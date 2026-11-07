//! `#size(T)` and `#alignment(T)`: the bytes a value of `T` takes and the address multiple it
//! sits on, known at check time like `#target.register_bits`.
//!
//! A domain takes the fewest whole machine bytes (1, 2, 4, 8, 16, 32) its range needs, signed
//! when it reaches below zero; a pointer takes a register; a record places its fields in order,
//! each on its own alignment, and rounds up to the largest. For a type parameter nothing is
//! known yet: the query is some positive number, and a bound that must hold for every type has
//! to hold for all of them. Sums and arrays are not defined here yet.
use super::range_bounds::bytes_for;
use super::*;

/// The largest size or alignment a type parameter's query may be.
const UNKNOWN_MAX: i64 = 1 << 62;

impl<'a> Checker<'a> {
    /// `(size, alignment)` of a type with no parameters left to find, or `None` when this
    /// checker does not define the layout of its kind.
    pub(super) fn layout_of(&mut self, ty: SourceType) -> Result<Option<(I256, I256)>> {
        self.charge(4)?;
        match ty {
            SourceType::Unit => Ok(Some((I256::ZERO, I256::ONE))),
            SourceType::Integer { lower, upper } => {
                let bytes = I256::from(bytes_for(lower, upper));
                Ok(Some((bytes, bytes)))
            }
            SourceType::Pointer { .. } => {
                let bytes = I256::from(self.target.register_bits / 8);
                Ok(Some((bytes, bytes)))
            }
            SourceType::Aggregate { id } => {
                let instance = self.module.instances[id as usize].clone();
                let schema = instance.schema();
                if let Some((lower, upper)) = self.domain_bounds_of(ty) {
                    return self.layout_of(SourceType::Integer { lower, upper });
                }
                let fields: Vec<SourceType> = self.module.layouts[schema]
                    .schema
                    .fields
                    .iter()
                    .map(|field| field.ty)
                    .collect();
                let (mut size, mut alignment) = (I256::ZERO, I256::ONE);
                for field in fields {
                    let field = self.substitute(field, &instance.arguments)?;
                    let Some((field_size, field_alignment)) = self.layout_of(field)? else {
                        return Ok(None);
                    };
                    let rest = size % field_alignment;
                    if rest != I256::ZERO {
                        size = size + field_alignment - rest;
                    }
                    size = size + field_size;
                    alignment = alignment.max(field_alignment);
                }
                let rest = size % alignment;
                if rest != I256::ZERO {
                    size = size + alignment - rest;
                }
                Ok(Some((size, alignment)))
            }
            _ => Ok(None),
        }
    }

    /// The value of `#size(T)` (`alignment` false) or `#alignment(T)`.
    pub(super) fn layout_query(&mut self, ty: SourceType, alignment: bool) -> Result<Value> {
        if let SourceType::Generic { index } = ty {
            // One identity per query, so `count * #size(x)` written twice is one number.
            self.charge(self.proof.layout_ids.len() + 1)?;
            if let Some((_, _, identity)) = self
                .proof
                .layout_ids
                .iter()
                .find(|(found, wanted, _)| *found == index && *wanted == alignment)
                .copied()
            {
                let known = self.proof.scalars.iter().any(|(id, _, _)| *id == identity);
                if known {
                    let mut value = Value::integer(I256::ONE, I256::from(UNKNOWN_MAX));
                    value.identity = Some(identity);
                    return self.scalar_facts(value);
                }
            }
            let fresh = self.ensure_identity(Value::integer(I256::ONE, I256::from(UNKNOWN_MAX)))?;
            if let Some(identity) = fresh.identity {
                self.proof.layout_ids.retain(|(found, wanted, _)| !(*found == index && *wanted == alignment));
                self.proof.layout_ids.push((index, alignment, identity));
            }
            return Ok(fresh);
        }
        let Some((size, aligned)) = self.layout_of(ty)? else {
            let detail = "the layout of this kind of type is not defined yet".to_owned();
            return Err(self.explain(SourceCheckError::Unsupported, detail));
        };
        let number = if alignment { aligned } else { size };
        Ok(Value::integer(number, number))
    }
}

/// `(size, alignment)` in bytes of a type of an instantiated file, by the same rule as
/// `layout_of`: a domain takes the fewest whole bytes its range needs, a pointer a register
/// (8), a record its fields in order each on its alignment. `None` where the checker defines
/// no layout (sums, arrays).
pub fn layout_in_file(file: &CheckedSourceFile, ty: SourceType) -> Option<(u64, u64)> {
    match ty {
        SourceType::Unit => Some((0, 1)),
        SourceType::Integer { lower, upper } => {
            let bytes = u64::from(bytes_for(lower, upper));
            Some((bytes, bytes))
        }
        SourceType::Pointer { .. } => Some((8, 8)),
        SourceType::Aggregate { id } => {
            let CheckedSourceInstanceHead::Nominal { schema } = file.instances.get(id as usize)?.head else {
                return None;
            };
            let aggregate = file.aggregates.get(schema as usize)?;
            if aggregate.kind == AggregateKind::Domain {
                return layout_in_file(file, aggregate.fields.first()?.ty);
            }
            let (mut size, mut alignment) = (0u64, 1u64);
            for field in &aggregate.fields {
                let (field_size, field_alignment) = layout_in_file(file, field.ty)?;
                size = size.next_multiple_of(field_alignment) + field_size;
                alignment = alignment.max(field_alignment);
            }
            Some((size.next_multiple_of(alignment), alignment))
        }
        _ => None,
    }
}

/// A node that is the number `value`, and its type.
pub fn integer_node(value: u64) -> (CheckedSourceBodyKind, SourceType) {
    let number = I256::from(value);
    (CheckedSourceBodyKind::Integer(number), SourceType::Integer { lower: number, upper: number })
}

/// The range `left <operator> right` takes for operands of these (instantiated) types, for a sum,
/// difference or product; `None` for anything else. A template's arithmetic was typed with the
/// widest range a type parameter's size may have; once the size is known the range narrows.
pub fn arithmetic_type(operator: CheckedArithmetic, left: SourceType, right: SourceType) -> Option<SourceType> {
    let (SourceType::Integer { lower: a, upper: b }, SourceType::Integer { lower: c, upper: d }) = (left, right) else {
        return None;
    };
    if !matches!(operator, CheckedArithmetic::Add | CheckedArithmetic::Subtract | CheckedArithmetic::Multiply) {
        return None;
    }
    let (lower, upper) = eval::interval(operator, (a, b), (c, d))?;
    Some(SourceType::Integer { lower, upper })
}
