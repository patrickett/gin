//! `deref value`: a copy of what a borrow refers to.
//!
//! A copy of a number is the number. A copy of a record is a new owner with the same contents,
//! allowed only when the record can be copied at all (`#unrestricted`); a linear value would be
//! duplicated along with its obligation, so it cannot be moved out of a borrow.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn copy_out(&mut self, borrowed: Value) -> Result<Value> {
        self.charge(1)?;
        match borrowed.ty {
            SourceType::Integer { .. } => Ok(Value {
                owner: None,
                available: true,
                borrow: None,
                ..borrowed
            }),
            SourceType::Aggregate { .. } | SourceType::Sum { .. } => {
                let owner = borrowed.owner.ok_or(SourceCheckError::Type)?;
                if !borrowed.available {
                    return Err(SourceCheckError::UseAfterMove);
                }
                // A type's own `copy` is where a linear value is duplicated: it is how the type
                // grants copying, so `deref self` there is the primitive.
                let granting = self.proof.in_copy && Some(borrowed.ty) == self.proof.receiver;
                if !self.proof.owners[owner].unrestricted && !granting {
                    return Err(SourceCheckError::MoveOutOfBorrow);
                }
                let copy = self.clone_owner(owner)?;
                self.proof.owners[copy].passing = calls::Passing::Owner;
                Ok(Value {
                    owner: Some(copy),
                    available: true,
                    borrow: None,
                    ..borrowed
                })
            }
            _ => Err(SourceCheckError::Type),
        }
    }
}

impl<'a> Checker<'a> {
    /// `#target.<field>`. The compiler reports only primitive facts of the machine; the rest is
    /// left to be worked out in Gin from them and from the ranges types declare.
    pub(super) fn target_fact(
        &mut self,
        expression: &'a Expression,
        base: &'a Expression,
        name: &'a Name,
    ) -> Result<Value> {
        let Expr::Query { hash, name: query } = &base.value else {
            return Err(SourceCheckError::Malformed);
        };
        self.child(expression.span_id, base.span_id)?;
        self.child(expression.span_id, name.span_id)?;
        self.child(base.span_id, *hash)?;
        self.child(base.span_id, query.span_id)?;
        self.token(*hash, "#")?;
        self.name(query)?;
        self.name(name)?;
        self.charge(4)?;
        if query.value != "target" {
            return Err(SourceCheckError::Unsupported);
        }
        match name.value.as_str() {
            "triple" => self.fresh_value(SourceType::Text),
            "register_bits" => {
                let bits = I256::from(self.target.register_bits);
                Ok(Value::integer(bits, bits))
            }
            _ => Err(SourceCheckError::UnknownName),
        }
    }
}
