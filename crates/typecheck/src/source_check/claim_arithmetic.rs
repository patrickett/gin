//! Arithmetic in claims: `capacity * #size(element) <= max_count`.
//!
//! A term may add, subtract or multiply other terms, and may ask the size or alignment of a type.
//! For a type that is still a type parameter the answer is only known to lie in an interval, so a
//! claim that depends on it is proved only if it holds for every number in that interval and
//! otherwise has to be restated by the caller; at a concrete instance the number is known and the
//! claim proves. Assuming such a claim narrows only through the intervals.
use super::contracts::{Constraint, Term};
use super::*;

impl<'a> Checker<'a> {
    /// The value a claim's term stands for.
    pub(super) fn term_value(&mut self, term: &Term, values: &mut [Value]) -> Result<Value> {
        self.charge(1)?;
        Ok(match term {
            Term::Slot(slot) => values[*slot],
            Term::Constant(number) => Value::integer(*number, *number),
            Term::Number(index) => self.number_term_value(*index)?,
            Term::Identity(identity) => self.identity_value(*identity)?,
            Term::Path { slot, field, deeper } => {
                let mut value = self.field_child(values[*slot], *field)?;
                for next in deeper {
                    value = self.field_child(value, *next)?;
                }
                value
            }
            Term::Arithmetic(operator, left, right) => {
                let left = self.term_value(left, values)?;
                let right = self.term_value(right, values)?;
                let left = self.open_number(left)?;
                let right = self.open_number(right)?;
                // An interval, with no identity of its own: nothing is related to its operands.
                let (lower, upper) = eval::interval(
                    *operator,
                    (left.lower, left.upper),
                    (right.lower, right.upper),
                )
                .ok_or(SourceCheckError::Overflow)?;
                let result = Value::integer(lower, upper);
                self.relate_arithmetic(*operator, left, right, result)?
            }
            Term::Layout { alignment, ty } => self.layout_query(*ty, *alignment)?,
        })
    }

    /// `term` with the type arguments of a call or instance put in: a size of a type parameter
    /// becomes the size of the type given, and a number parameter inside is replaced as it is at
    /// top level (`number_constraints`).
    pub(super) fn substitute_term(&mut self, term: &Term, types: &[SourceType]) -> Result<Term> {
        self.charge(1)?;
        Ok(match term {
            Term::Arithmetic(operator, left, right) => Term::Arithmetic(
                *operator,
                Box::new(self.substitute_term(left, types)?),
                Box::new(self.substitute_term(right, types)?),
            ),
            Term::Layout { alignment, ty } => {
                let ty = self.substitute(*ty, types)?;
                Term::Layout { alignment: *alignment, ty }
            }
            Term::Number(index) => match types.get(*index as usize) {
                Some(SourceType::Integer { lower, upper }) if lower == upper => {
                    Term::Constant(*lower)
                }
                _ => term.clone(),
            },
            other => other.clone(),
        })
    }

    /// `left + right`, `left - right`, `left * right`, or `#size(T)` / `#alignment(T)` as a term.
    pub(super) fn arithmetic_term(
        &mut self,
        expression: &'a Expression,
        names: &[&str],
    ) -> Result<Option<Term>> {
        match &expression.value {
            Expr::Binary { left, operator, right } => {
                let Some(arithmetic) =
                    InfixOperator::from_binary(operator.value).and_then(InfixOperator::arithmetic)
                else {
                    return Ok(None);
                };
                self.child(expression.span_id, left.span_id)?;
                self.child(expression.span_id, operator.span_id)?;
                self.child(expression.span_id, right.span_id)?;
                let left = self.contract_term(left, names)?;
                let right = self.contract_term(right, names)?;
                Ok(Some(Term::Arithmetic(arithmetic, Box::new(left), Box::new(right))))
            }
            Expr::Call { callee, arguments }
                if matches!(&callee.value, Expr::Query { name, .. }
                    if matches!(name.value.as_str(), "size" | "alignment")) =>
            {
                let Expr::Query { name, .. } = &callee.value else {
                    return Ok(None);
                };
                let [argument] = arguments.value.as_slice() else {
                    return Err(SourceCheckError::Type);
                };
                let Argument::Positional(written) = &argument.value else {
                    return Err(SourceCheckError::Type);
                };
                self.child(expression.span_id, callee.span_id)?;
                self.child(expression.span_id, arguments.span_id)?;
                let scope = self.generic_scope();
                let ty = self.resolve_type(written, &scope)?;
                let alignment = name.value == "alignment";
                match ty {
                    SourceType::Generic { .. } => Ok(Some(Term::Layout { alignment, ty })),
                    concrete => {
                        let value = self.layout_query(concrete, alignment)?;
                        Ok(Some(Term::Constant(value.lower)))
                    }
                }
            }
            _ => Ok(None),
        }
    }
}

#[allow(dead_code)]
fn _constraint_is_clone(constraint: &Constraint) -> Constraint {
    constraint.clone()
}
