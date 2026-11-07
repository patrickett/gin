//! Raw addresses of places: `place as @T` and `@place`.
//!
//! The address of a borrowed array or any other place is a `Pointer` value that is unchecked
//! from then on: no loan is recorded, so nothing keeps the place alive or unaliased while the
//! pointer exists. Reading or writing through it is possible only with library load/store
//! instructions. This is the trust boundary that `ref`/`mut` do not cross.
use super::*;

/// Whether `expression` is a call of the `#unchecked` directive, in any shape.
pub(super) fn is_unchecked(expression: &Expression) -> bool {
    matches!(&expression.value, Expr::Call { callee, .. }
        if matches!(&callee.value, Expr::Query { name, .. } if name.value == "unchecked"))
}

/// `#unchecked(value)`: the operand, when `expression` is the directive with exactly one
/// positional argument. Another shape of the directive is refused where it is evaluated.
pub(super) fn unchecked_operand(expression: &Expression) -> Option<&Expression> {
    if !is_unchecked(expression) {
        return None;
    }
    let Expr::Call { arguments, .. } = &expression.value else {
        return None;
    };
    let [argument] = arguments.value.as_slice() else {
        return None;
    };
    let Argument::Positional(value) = &argument.value else {
        return None;
    };
    Some(value)
}

impl<'a> Checker<'a> {
    /// An operation that stops being proved is allowed only inside `#unchecked(...)`. The
    /// innermost directive around it is the one that accepts it, so
    /// `#unchecked((@line) as @Byte)` accepts the address and the cast, while a function that
    /// holds an acceptance is not marked itself and its callers are not forced.
    pub(super) fn require_acceptance(&mut self, span: SpanId, operation: &str) -> Result<()> {
        self.charge(1)?;
        if let Some(entry) = self.acceptances.last_mut() {
            entry.1 = true;
            return Ok(());
        }
        self.culprit = Some(span);
        let detail = format!(
            "{operation} stops being checked; accept that by writing the expression inside `#unchecked(...)`"
        );
        Err(self.explain(SourceCheckError::MissingAcceptance, detail))
    }

    /// The span structure of `#unchecked(value)`, checked like any other query call: the
    /// directive with one positional argument, or an explained refusal.
    pub(super) fn unchecked_syntax(&mut self, expression: &'a Expression) -> Result<&'a Expression> {
        let Expr::Call { callee, arguments } = &expression.value else {
            return Err(SourceCheckError::Malformed);
        };
        let Expr::Query { hash, name } = &callee.value else {
            return Err(SourceCheckError::Malformed);
        };
        self.child(expression.span_id, callee.span_id)?;
        self.child(expression.span_id, arguments.span_id)?;
        self.child(callee.span_id, *hash)?;
        self.child(callee.span_id, name.span_id)?;
        self.token(*hash, "#")?;
        self.name(name)?;
        self.charge(4)?;
        let [argument] = arguments.value.as_slice() else {
            let detail = "`#unchecked` takes exactly one expression: `#unchecked(value)`".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        };
        self.child(arguments.span_id, argument.span_id)?;
        let Argument::Positional(value) = &argument.value else {
            let detail = "`#unchecked` takes an expression, not a named argument".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        };
        self.child(argument.span_id, value.span_id)?;
        Ok(value)
    }

    /// Leave the innermost marker, reporting whether an unchecked operation used it.
    pub(super) fn release_acceptance(&mut self, span: SpanId) -> Result<bool> {
        self.charge(1)?;
        match self.acceptances.pop() {
            Some((marked, used)) if marked == span => Ok(used),
            _ => Err(SourceCheckError::Malformed),
        }
    }

    /// `place as @T`, when `target` is a pointer type and `place` is a place that is not itself
    /// a pointer: the address of the place (or of the first element of an array place).
    /// `None` leaves the cast to the ordinary rules.
    pub(super) fn address_cast(
        &mut self,
        span: SpanId,
        place: &'a Expression,
        target: &'a Expression,
    ) -> Result<Option<Value>> {
        let Expr::PrefixedType(prefix) = &target.value else {
            return Ok(None);
        };
        if prefix.keyword.value != TypePrefixKind::Pointer {
            return Ok(None);
        }
        if !self.is_place(place)? {
            return Ok(None);
        }
        let place_value = self.borrow_place(place, false)?;
        if matches!(place_value.ty, SourceType::Pointer { .. }) {
            return Ok(None);
        }
        let SourceType::Pointer { id } = self.cast_target(target)? else {
            return Err(SourceCheckError::Type);
        };
        let pointee = self.module.instances[id as usize].arguments[0];
        let element = match place_value.ty {
            SourceType::Array { .. } => Some(self.array_layout(place_value.ty)?.0),
            _ => None,
        };
        if pointee != place_value.ty && Some(pointee) != element {
            // A number cast to a pointer of another type is that number as an address.
            if matches!(place_value.ty, SourceType::Integer { .. }) || self.is_open_domain(place_value.ty) {
                return Ok(None);
            }
            let detail = "an address can only be cast to a pointer to the place's own type, or to its element type".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        self.require_acceptance(span, "taking the address of a place with `as @T`")?;
        Ok(Some(self.fresh_value(SourceType::Pointer { id })?))
    }

    /// `@place`: the address of a place, typed as a pointer to the place's type.
    pub(super) fn address_expression(
        &mut self,
        span: SpanId,
        place: &'a Expression,
    ) -> Result<Value> {
        if !self.is_place(place)? {
            return Err(SourceCheckError::Unsupported);
        }
        self.require_acceptance(span, "taking the address of a place with `@place`")?;
        let value = self.borrow_place(place, false)?;
        let ty = self.pointer_type(value.ty)?;
        self.fresh_value(ty)
    }
}

impl<'a> Checker<'a> {
    /// `number as Domain`: the number viewed in the domain, accepted when its range is proved to
    /// fit. Nothing is converted at run time.
    pub(super) fn number_cast(&mut self, value: Value, target: SourceType) -> Result<Value> {
        let number = self.domain_number(value).map_err(|error| match error {
            SourceCheckError::Type => SourceCheckError::Type,
            other => other,
        })?;
        self.scalar(number)?;
        self.view_as(number, target)
    }
}
