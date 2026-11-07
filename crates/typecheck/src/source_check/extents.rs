//! Dependent extents: `data [element; len]` with `len` a sibling field.
//!
//! The type of such a field is an array of the `Slice` head: its extent is not part of the
//! type but a property of each value, kept in `Owner::extent` as the identity of the number it
//! equals. A record's `data` child gets the identity of its `len` child when the record is
//! materialized, so `self.len` read twice and `index < self.len` talk about one number, and the
//! ordinary fact machinery (`entails`) proves `index < len`. `base.[0; n]` makes a slice whose
//! extent is `n`, after proving `n` fits the source.
//!
//! v1 limits: a slice starts at the constant `0`; elements read through a slice are fresh
//! unknown values of the (unrestricted) element type; only slices of borrowed arrays (or any
//! `Slice` value) are indexed by a variable, so owned fixed arrays keep their exact slots.
use super::*;

/// How long an array value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Extent {
    /// Known from the type.
    Const(u32),
    /// Equal to the number with this identity.
    Scalar(u32),
}

impl<'a> Checker<'a> {
    pub(super) fn slice_type(&mut self, element: SourceType) -> Result<SourceType> {
        self.intern_instance(CheckedSourceInstanceHead::Slice, vec![element])
    }

    /// Whether `ty` is written `[element; name]` with `name` a sibling field (not a type
    /// parameter). Uncharged: it only looks at the shape.
    pub(super) fn names_sibling_length(
        ty: &Expression,
        scope: &[(&str, SourceType)],
        siblings: &[&str],
    ) -> bool {
        let Expr::Array(array) = &ty.value else {
            return false;
        };
        let Some(ArrayLength { value, .. }) = &array.length else {
            return false;
        };
        let Expr::Name(name) = &value.value else {
            return false;
        };
        !scope.iter().any(|(other, _)| other == name) && siblings.iter().any(|other| other == name)
    }

    /// `[element; len]` where `len` names a field of the same record: the element type and the
    /// sibling's position. `None` when `ty` is not written that way.
    pub(super) fn dependent_array(
        &mut self,
        ty: &'a Expression,
        scope: &[(&'a str, SourceType)],
        siblings: &[&str],
    ) -> Result<Option<(SourceType, u32)>> {
        let Expr::Array(array) = &ty.value else {
            return Ok(None);
        };
        let Some(length) = &array.length else {
            return Ok(None);
        };
        let Expr::Name(name) = &length.value.value else {
            return Ok(None);
        };
        if scope.iter().any(|(other, _)| *other == name) {
            return Ok(None);
        }
        let Some(position) = siblings.iter().position(|other| *other == name) else {
            return Ok(None);
        };
        self.array_syntax(ty, array)?;
        self.token(length.value.span_id, name)?;
        if array.elements.len() != 1 {
            return Err(SourceCheckError::Type);
        }
        let element = self.resolve_type(&array.elements[0], scope)?;
        Ok(Some((self.slice_type(element)?, position as u32)))
    }

    /// Every dependent array names an integer field of its record.
    pub(super) fn check_length_fields(&mut self, fields: &[CheckedSourceField]) -> Result<()> {
        for field in fields {
            if let Some(position) = field.length {
                let sibling = fields
                    .get(position as usize)
                    .ok_or(SourceCheckError::Type)?;
                // A number, or a named integer domain (`len Length`): then the length is the
                // number the domain holds.
                let named_domain = matches!(sibling.ty, SourceType::Aggregate { id }
                    if self.module.layouts[self.module.instances[id as usize].schema()].schema.kind
                        == AggregateKind::Domain);
                if !matches!(sibling.ty, SourceType::Integer { .. }) && !named_domain {
                    let detail = format!(
                        "`{}` is the length of `{}` but is not a number",
                        sibling.name, field.name
                    );
                    return Err(self.explain(SourceCheckError::Type, detail));
                }
            }
        }
        Ok(())
    }

    /// Give each dependent array of a record the identity of its length child.
    pub(super) fn set_input_extents(&mut self, id: u32, children: &[Value]) {
        let schema = self.module.instances[id as usize].schema();
        for (index, child) in children.iter().enumerate() {
            let Some(position) = self.module.layouts[schema].schema.fields[index].length else {
                continue;
            };
            // A length held in a named domain is the number inside it.
            let identity = children
                .get(position as usize)
                .copied()
                .and_then(|sibling| self.domain_number(sibling).ok())
                .and_then(|number| number.identity);
                if let (Some(owner), Some(identity)) = (child.owner, identity) {
                self.proof.owners[owner].extent = Some(identity);
            }
        }
    }

    pub(super) fn extent_of(&mut self, value: Value) -> Result<Extent> {
        self.charge(1)?;
        let SourceType::Array { id } = value.ty else {
            return Err(SourceCheckError::Type);
        };
        if let Some(identity) = value
            .owner
            .and_then(|owner| self.proof.owners[owner].extent)
        {
            return Ok(Extent::Scalar(identity));
        }
        match self.module.instances[id as usize].head {
            CheckedSourceInstanceHead::Array { extent } => Ok(Extent::Const(extent)),
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    fn scalar_bounds(&mut self, identity: u32) -> Result<(I256, I256)> {
        self.charge(self.proof.scalars.len())?;
        self.proof
            .scalars
            .iter()
            .find(|(id, _, _)| *id == identity)
            .map(|(_, lower, upper)| (*lower, *upper))
            .ok_or(SourceCheckError::Type)
    }

    /// Whether `number < extent` is known.
    pub(super) fn proves_below(&mut self, number: Value, extent: Extent) -> Result<bool> {
        let number = self.scalar_facts(number)?;
        match extent {
            Extent::Const(length) => Ok(number.upper < I256::from(length)),
            Extent::Scalar(identity) => {
                let (lower, _) = self.scalar_bounds(identity)?;
                if number.upper < lower {
                    return Ok(true);
                }
                match number.identity {
                    Some(own) => Ok(self.entails(BinaryOperator::Less, own, identity)?
                        || self.linear_entails(BinaryOperator::Less, own, identity)?),
                    None => Ok(false),
                }
            }
        }
    }

    /// Whether `number <= extent` is known.
    fn proves_at_most(&mut self, number: Value, extent: Extent) -> Result<bool> {
        let number = self.scalar_facts(number)?;
        match extent {
            Extent::Const(length) => Ok(number.upper <= I256::from(length)),
            Extent::Scalar(identity) => {
                let (lower, _) = self.scalar_bounds(identity)?;
                if number.upper <= lower {
                    return Ok(true);
                }
                match number.identity {
                    Some(own) if own == identity => Ok(true),
                    Some(own) => Ok(self.entails(BinaryOperator::LessOrEqual, own, identity)?
                        || self.linear_entails(BinaryOperator::LessOrEqual, own, identity)?),
                    None => Ok(false),
                }
            }
        }
    }

    /// Whether `length` is known to be exactly `extent`.
    fn proves_equal(&mut self, length: Value, extent: Extent) -> Result<bool> {
        let length = self.scalar_facts(length)?;
        match extent {
            Extent::Const(count) => {
                Ok(length.lower == length.upper && length.lower == I256::from(count))
            }
            Extent::Scalar(identity) => {
                if length.identity == Some(identity) {
                    return Ok(true);
                }
                let (lower, upper) = self.scalar_bounds(identity)?;
                if lower == upper && length.lower == length.upper && length.lower == lower {
                    return Ok(true);
                }
                match length.identity {
                    Some(own) => self.entails(BinaryOperator::Equal, own, identity),
                    None => Ok(false),
                }
            }
        }
    }

    /// A record's dependent arrays are as long as the sibling fields say.
    pub(super) fn lengths_agree(&mut self, schema: usize, children: &[Value]) -> Result<()> {
        for index in 0..children.len() {
            let Some(position) = self.module.layouts[schema].schema.fields[index].length else {
                continue;
            };
            let field = self.module.layouts[schema].schema.fields[index]
                .name
                .clone();
            let length = self.module.layouts[schema].schema.fields[position as usize]
                .name
                .clone();
            let extent = self.extent_of(children[index])?;
            let held = self.domain_number(children[position as usize])?;
            if !self.proves_equal(held, extent)? {
                let detail = format!("`{field}` is not proved to have length `{length}`");
                return Err(self.explain(SourceCheckError::Type, detail));
            }
        }
        Ok(())
    }

    /// `ref base.[0; n]` / `mut base.[0; n]`: borrow the array `base` and view its first `n`
    /// elements. The loan is on the whole of `base`'s root.
    pub(super) fn slice_handle(
        &mut self,
        expression: &'a Expression,
        select: &'a SelectExpression,
        exclusive: bool,
    ) -> Result<Value> {
        self.child(expression.span_id, select.base.span_id)?;
        self.child(expression.span_id, select.index.span_id)?;
        for (span, token) in [(select.dot, "."), (select.open, "["), (select.close, "]")] {
            self.child(expression.span_id, span)?;
            self.token(span, token)?;
        }
        let length = select
            .length
            .as_ref()
            .ok_or(SourceCheckError::Unsupported)?;
        self.child(expression.span_id, length.separator)?;
        self.child(expression.span_id, length.value.span_id)?;
        self.token(length.separator, ";")?;
        let start = self.expression(&select.index)?;
        let start = self.domain_number(start)?;
        self.scalar(start)?;
        if start.lower < I256::ZERO {
            let detail = "the start of a sub-view is not shown to be at least 0".to_owned();
            return Err(self.explain(SourceCheckError::Unproved, detail));
        }
        let zero = start.lower == I256::ZERO && start.upper == I256::ZERO;
        let count = self.expression(&length.value)?;
        let count = self.domain_number(count)?;
        self.scalar(count)?;
        let count = self.ensure_identity(count)?;
        let source = self.borrow_handle(&select.base, exclusive)?;
        let Some(handle) = source.borrow else {
            return Err(SourceCheckError::Unsupported);
        };
        let SourceType::Array { .. } = source.ty else {
            return Err(SourceCheckError::Type);
        };
        let extent = self.extent_of(source)?;
        let count = self.scalar_facts(count)?;
        // The sub-view ends inside the array: `start + count <= extent`, related to its operands.
        let reach = if zero {
            count
        } else {
            let start = self.ensure_identity(start)?;
            self.proof.relate_always = true;
            let sum = self.arithmetic(CheckedArithmetic::Add, start, count);
            self.proof.relate_always = false;
            self.ensure_identity(sum?)?
        };
        if count.lower < I256::ZERO || !self.proves_at_most(reach, extent)? {
            let detail = "cannot prove the slice length is within the array".to_owned();
            return Err(self.explain(SourceCheckError::Unproved, detail));
        }
        let (element, _) = self.array_layout(source.ty)?;
        let ty = self.slice_type(element)?;
        let mut value = self.fresh_value(ty)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner].extent = count.identity;
        self.proof.owners[owner].passing = if exclusive {
            calls::Passing::Mut
        } else {
            calls::Passing::Ref
        };
        value.borrow = Some(handle);
        Ok(value)
    }

    /// `base.[index]` on a borrowed or sliced array: a read of one element, accepted when
    /// `0 <= index < extent` is proved. The element is a fresh unknown value of its type.
    pub(super) fn dynamic_select(
        &mut self,
        select: &'a SelectExpression,
        index: Value,
    ) -> Result<Value> {
        let array = self.project_place(&select.base)?;
        let SourceType::Array { id } = array.ty else {
            return Err(SourceCheckError::Unsupported);
        };
        let dynamic = matches!(
            self.module.instances[id as usize].head,
            CheckedSourceInstanceHead::Slice
                | CheckedSourceInstanceHead::Sized
                | CheckedSourceInstanceHead::Array { .. }
        ) || array.borrow.is_some()
            || array
                .owner
                .is_some_and(|owner| self.proof.owners[owner].passing.borrows());
        if !dynamic {
            return Err(SourceCheckError::Unsupported);
        }
        let (element, _) = self.array_layout(array.ty)?;
        if self.capability_of(element) != Capability::Unrestricted {
            // A linear element cannot be copied out of a borrowed array.
            return Err(SourceCheckError::MoveOutOfBorrow);
        }
        let extent = self.extent_of(array)?;
        let index = self.scalar_facts(index)?;
        if index.lower < I256::ZERO || !self.proves_below(index, extent)? {
            let detail = "cannot prove the index is within the array".to_owned();
            return Err(self.explain(SourceCheckError::Unproved, detail));
        }
        self.fresh_value(element)
    }
}

impl<'a> Checker<'a> {
    /// `#unchecked(address as ref [item; n])` (or `mut`): the array of `n` elements that
    /// starts at `address`. The one trusted step from a number-derived origin to an array; the
    /// rest of the program indexes it with the ordinary bounds proof (`index < n`).
    ///
    /// * the address is read from a place (a field of the receiver, a local), and the array
    ///   borrows that place's root: while it lives nothing can use the root in a conflicting way;
    /// * `mut` needs a place that may be lent `mut`, so a `ref` view is never turned into one;
    /// * the source is a pointer or an unrestricted domain over a number (an origin), never a
    ///   plain number or a linear domain;
    /// * `n` is a number in scope; the array's extent is that number.
    ///
    /// `None` leaves any other cast to the ordinary rules.
    pub(super) fn view_cast(
        &mut self,
        expression: &'a Expression,
        value: &'a Expression,
        target: &'a Expression,
    ) -> Result<Option<Value>> {
        let Expr::PrefixedType(prefixed) = &target.value else {
            return Ok(None);
        };
        let Expr::Array(array) = &prefixed.target.value else {
            return Ok(None);
        };
        if !matches!(prefixed.keyword.value, TypePrefixKind::Ref | TypePrefixKind::Mut)
            || prefixed.group.is_some()
            || array.length.is_none()
            || array.elements.len() != 1
        {
            return Ok(None);
        }
        let Expr::Cast { as_keyword, .. } = &expression.value else {
            return Ok(None);
        };
        self.cast_syntax(expression, value, *as_keyword, target)?;
        let exclusive = prefixed.keyword.value == TypePrefixKind::Mut;
        self.keyword(prefixed.keyword.span_id, if exclusive { "mut" } else { "ref" })?;
        self.child(target.span_id, prefixed.keyword.span_id)?;
        self.child(target.span_id, prefixed.target.span_id)?;
        self.array_syntax(&prefixed.target, array)?;
        self.charge(4)?;
        self.require_acceptance(expression.span_id, "viewing an address as an array")?;
        if !self.is_place(value)? {
            let detail = "the address of a view is read from a place, so the view can borrow it".to_owned();
            return Err(self.explain(SourceCheckError::Unsupported, detail));
        }
        let source = self.project_place(value)?;
        let origin = match source.ty {
            SourceType::Pointer { .. } => true,
            ty if self.is_open_domain(ty) => {
                self.capability_of(ty) == Capability::Unrestricted
                    && self.domain_bounds_of(ty).is_some_and(|(lower, upper)| {
                        lower >= I256::ZERO && upper <= I256::from(u64::MAX)
                    })
            }
            _ => false,
        };
        if !origin {
            let detail = "only an address (a pointer or an unrestricted origin domain) can be viewed as an array".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        let element = self.cast_target(&array.elements[0])?;
        let length = array.length.as_ref().ok_or(SourceCheckError::Malformed)?;
        let count = self.expression(&length.value)?;
        let count = self.domain_number(count)?;
        self.scalar(count)?;
        let count = self.ensure_identity(count)?;
        let count = self.scalar_facts(count)?;
        if count.lower < I256::ZERO {
            let detail = "cannot prove the length of the view is not negative".to_owned();
            return Err(self.explain(SourceCheckError::Unproved, detail));
        }
        let handle = self.borrow_handle(value, exclusive)?;
        let Some(handle) = handle.borrow else {
            return Err(SourceCheckError::Unsupported);
        };
        let ty = self.slice_type(element)?;
        let mut view = self.fresh_value(ty)?;
        let owner = view.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner].extent = count.identity;
        self.proof.owners[owner].passing = if exclusive {
            calls::Passing::Mut
        } else {
            calls::Passing::Ref
        };
        view.borrow = Some(handle);
        Ok(Some(view))
    }
}

impl<'a> Checker<'a> {
    /// A text literal where a read-only view of bytes is wanted: `len` and `data ref [Byte; len]`,
    /// the shape of `SliceView(Byte)`. The literal is the one array whose address and exact length
    /// the compiler knows, so no acceptance is needed: the view's length is the literal's byte
    /// length, which the value carries, and the data is read-only. A `mut` view, a view of
    /// another element type, or a text that is not a literal (no length) is not this.
    /// Whether `ty` is the shape a literal can be viewed as: `len` and `data ref [Byte; len]`.
    pub(super) fn is_byte_view(&self, ty: SourceType) -> bool {
        let SourceType::Aggregate { id } = ty else {
            return false;
        };
        let schema = self.module.instances[id as usize].schema();
        let fields = &self.module.layouts[schema].schema.fields;
        fields.len() == 2
            && fields[1].length == Some(0)
            && fields[1].borrow == Some(BorrowKind::Shared)
            && fields[0].borrow.is_none()
    }

    pub(super) fn text_as_bytes(&mut self, text: Value, expected: SourceType) -> Result<Option<Value>> {
        let SourceType::Aggregate { id } = expected else {
            return Ok(None);
        };
        let schema = self.module.instances[id as usize].schema();
        let fields = &self.module.layouts[schema].schema.fields;
        let shaped = fields.len() == 2
            && fields[1].length == Some(0)
            && fields[1].borrow == Some(BorrowKind::Shared)
            && fields[0].borrow.is_none();
        if !shaped {
            return Ok(None);
        }
        // Only a literal carries its length.
        let Some((length, _)) = text.integer_domain.filter(|(low, high)| low == high && *low == text.lower) else {
            return Err(SourceCheckError::Type);
        };
        let typed = self.instance_fields(id)?;
        let (SourceType::Array { .. }, true) = (typed[1], true) else {
            return Err(SourceCheckError::Type);
        };
        let (element, _) = self.array_layout(typed[1])?;
        // Bytes: a domain over 0 to 255.
        if self.domain_bounds_of(element) != Some((I256::ZERO, I256::from(255u32))) {
            return Err(SourceCheckError::Type);
        }
        let mut count = Value::integer(length, length);
        count = self.ensure_identity(count)?;
        let mut data = self.fresh_value(typed[1])?;
        let owner = data.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner].extent = count.identity;
        self.proof.owners[owner].passing = calls::Passing::Ref;
        data.borrow = None;
        let view = self.pack_indexed(schema, Some(expected), vec![0, 1], vec![count, data])?;
        Ok(Some(view))
    }
}
