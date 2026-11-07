//! Ranges as type arguments and the compile-time queries `#lower(T)` and `#upper(T)`.
//!
//! The compiler knows no `.min` or `.max` of its own for a user type: `domain of T` and
//! `a to b` are range values, a type argument takes one (`Range(domain of Byte)`), and the
//! library writes `min` and `max` itself. The one primitive it offers for that is reading
//! the ends of a range: `#lower(r)` and `#upper(r)` are the integers a range type starts and
//! ends at, known at check time, like `#target.register_bits`.
//!
//! Inside a generic body `r` is not known, so the query is some value of `r` (it is in the
//! range). A function whose whole body is one such query is marked, so a call that knows the
//! range (`Range(domain of Byte).max()`) answers with the exact number.
use super::*;

/// `#lower(T)` or `#upper(T)`, and the type parameter it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BoundQuery {
    pub upper: bool,
    pub generic: usize,
}

/// The whole bytes (1, 2, 4, 8, 16, 32) a range needs: signed when it reaches below zero.
pub(super) fn bytes_for(lower: I256, upper: I256) -> u32 {
    let bits = if lower < I256::ZERO {
        let below = 256 - (-lower - I256::ONE).leading_zeros();
        let above = 256 - upper.max(I256::ZERO).leading_zeros();
        below.max(above) + 1
    } else {
        256 - upper.leading_zeros()
    };
    let mut bytes = 1;
    while bytes * 8 < bits {
        bytes *= 2;
    }
    bytes
}

impl<'a> Checker<'a> {
    /// `domain of T`: the range `T` declares, as a type.
    pub(super) fn domain_type(&mut self, ty: SourceType) -> Result<SourceType> {
        self.charge(2)?;
        match ty {
            SourceType::Integer { .. } => Ok(ty),
            other => {
                let (lower, upper) = self
                    .domain_bounds_of(other)
                    .ok_or(SourceCheckError::Type)?;
                Ok(SourceType::Integer { lower, upper })
            }
        }
    }

    /// `a to b` where a type argument goes: the integers from `a` through `b`.
    pub(super) fn range_type(
        &mut self,
        expression: &'a Expression,
        start: &'a Expression,
        to_keyword: SpanId,
        end: &'a Expression,
    ) -> Result<SourceType> {
        self.child(expression.span_id, start.span_id)?;
        self.child(expression.span_id, to_keyword)?;
        self.child(expression.span_id, end.span_id)?;
        self.token(to_keyword, "to")?;
        self.charge(4)?;
        let (SourceType::Integer { lower, .. }, SourceType::Integer { lower: upper, .. }) =
            (self.number_argument(start)?, self.number_argument(end)?)
        else {
            return Err(SourceCheckError::Malformed);
        };
        if lower > upper {
            let detail = format!("the range {lower} to {upper} is empty");
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        Ok(SourceType::Integer { lower, upper })
    }

    /// `#lower(T)` or `#upper(T)`.
    pub(super) fn bound_query(
        &mut self,
        expression: &'a Expression,
        callee: &'a Expression,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
    ) -> Result<Value> {
        let Expr::Query { hash, name: query } = &callee.value else {
            return Err(SourceCheckError::Malformed);
        };
        self.child(expression.span_id, callee.span_id)?;
        self.child(expression.span_id, arguments.span_id)?;
        self.child(callee.span_id, *hash)?;
        self.child(callee.span_id, query.span_id)?;
        self.token(*hash, "#")?;
        self.name(query)?;
        self.charge(4)?;
        let upper = match query.value.as_str() {
            "lower" => false,
            "upper" => true,
            "size" | "alignment" => true,
            _ => return Err(SourceCheckError::Unsupported),
        };
        let layout = matches!(query.value.as_str(), "size" | "alignment");
        let [argument] = arguments.value.as_slice() else {
            return Err(SourceCheckError::Type);
        };
        self.child(arguments.span_id, argument.span_id)?;
        let Argument::Positional(written) = &argument.value else {
            return Err(SourceCheckError::Type);
        };
        self.child(argument.span_id, written.span_id)?;
        let scope = self.generic_scope();
        // What a query reads is a type: a value (`#size(x)`) is not one.
        let ty = self.resolve_type(written, &scope).map_err(|error| match error {
            SourceCheckError::UnknownName => SourceCheckError::Unsupported,
            other => other,
        })?;
        if layout {
            return self.layout_query(ty, query.value == "alignment");
        }
        // A declared domain stands for the range it holds.
        let ty = match self.domain_bounds_of(ty) {
            Some((lower, upper)) => SourceType::Integer { lower, upper },
            None => ty,
        };
        match ty {
            SourceType::Integer { lower, upper: top } => {
                let bound = if upper { top } else { lower };
                Ok(Value::integer(bound, bound))
            }
            // A range not known yet: some value inside it.
            generic @ SourceType::Generic { .. } => {
                let value = self.fresh_value(generic)?;
                // The compiler makes it, as the machine makes an `#instruction` result.
                if let Some(owner) = value.owner {
                    self.proof.owners[owner].drained = true;
                }
                Ok(value)
            }
            _ => {
                let detail = format!("`#{}` reads the ends of a range", query.value);
                Err(self.explain(SourceCheckError::Type, detail))
            }
        }
    }

    /// The query a function body is made of, when it is nothing else: `return #upper(r)`.
    pub(super) fn bound_query_of(&self, binding: &Binding) -> Option<BoundQuery> {
        let mut expression = &binding.value.as_ref()?.expression;
        if let Expr::Block(block) = &expression.value {
            let [statement] = block.statements.as_slice() else {
                return None;
            };
            let Statement::Return(ReturnStatement { value: Some(value) }) = &statement.value
            else {
                return None;
            };
            expression = value;
        }
        let Expr::Call { callee, arguments } = &expression.value else {
            return None;
        };
        let Expr::Query { name, .. } = &callee.value else {
            return None;
        };
        let upper = match name.value.as_str() {
            "lower" => false,
            "upper" => true,
            _ => return None,
        };
        let [argument] = arguments.value.as_slice() else {
            return None;
        };
        let Argument::Positional(Spanned {
            value: Expr::Name(parameter),
            ..
        }) = &argument.value
        else {
            return None;
        };
        let generic = self
            .proof
            .generics
            .iter()
            .position(|name| *name == parameter.as_str())?;
        Some(BoundQuery { upper, generic })
    }

    /// What a call of a bound-query function answers when the range is known.
    pub(super) fn known_bound(
        &self,
        query: Option<BoundQuery>,
        types: &[SourceType],
    ) -> Option<Value> {
        let query = query?;
        let SourceType::Integer { lower, upper } = *types.get(query.generic)? else {
            return None;
        };
        let bound = if query.upper { upper } else { lower };
        Some(Value::integer(bound, bound))
    }
}
