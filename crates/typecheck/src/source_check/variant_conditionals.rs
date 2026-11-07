//! `when subject is Pattern then A else B` as an expression: a conditional on a variant test.
//!
//! The subject's member is a replay choice, as in a full case: in the world where it is the
//! pattern's variant, `A` is the value and the pattern's binders (`_` discards) scope over it;
//! in every other world `B` is. `not subject is Pattern` swaps them and binds nothing. A named
//! subject is read in place, so the world that passes over the pattern still has it, narrowed;
//! an unnamed one is a temporary and is discarded there, which a value that must be consumed
//! cannot be.
use super::*;
use aggregates::EvalTask;

impl<'a> Checker<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn variant_conditional(
        &mut self,
        expression: &'a Expression,
        case: &'a ConditionalExpression,
        negated: bool,
        test: &'a IsTest,
        expected: Option<SourceType>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let MatchPattern::Variant(pattern) = &test.pattern.value else {
            return Err(SourceCheckError::Unsupported);
        };
        if negated && pattern.fields.as_ref().is_some_and(|f| !f.binders.value.is_empty()) {
            // Nothing binds where the pattern did not match.
            return Err(SourceCheckError::Unsupported);
        }
        let place = assign::split_place(&test.subject).is_some();
        if !visited {
            self.child(expression.span_id, case.condition.span_id)?;
            self.child(expression.span_id, case.then_branch.span_id)?;
            self.child(expression.span_id, case.else_branch.span_id)?;
            self.token(case.when_keyword, "when")?;
            self.token(case.then_keyword, "then")?;
            self.token(case.else_keyword, "else")?;
            self.condition_syntax(&case.condition)?;
            self.charge(3)?;
            pending.push(EvalTask::Expression(expression, expected, true));
            if !place {
                pending.push(EvalTask::Expression(&test.subject, None, false));
            }
            return Ok(());
        }
        let subject = if place {
            self.project_place(&test.subject)?
        } else {
            values.pop().ok_or(SourceCheckError::Malformed)?
        };
        let subject = self.inspect_value(subject)?;
        let variants = match subject.sum {
            Some(id) => self.sum_variants(id)?,
            None => vec![subject.ty],
        };
        let SourceType::Aggregate { id } = subject.ty else {
            return Err(SourceCheckError::Type);
        };
        let active = self.module.instances[id as usize].schema();
        let schema = self.pattern_schema(pattern, test.pattern.span_id, &variants)?;
        let matched = schema == active;
        if matched != negated {
            // The `then` branch. A matching pattern consumes the subject and binds its fields.
            if matched {
                self.charge(self.proof.names.len())?;
                let saved = self.proof.names.clone();
                let mut hidden = Vec::new();
                if let Some(fields) = &pattern.fields {
                    for binder in &fields.binders.value {
                        hidden.push(self.name(binder)?);
                    }
                }
                self.charge_product(self.proof.names.len(), hidden.len() + 1)?;
                self.proof.names.retain(|(name, _)| !hidden.contains(name));
                let linear = self.holds_linear(subject);
                self.bind_pattern(pattern, test.pattern.span_id, subject)?;
                let narrowing = self.narrows(pattern, schema);
                if narrowing
                    && let Expr::Name(name) = &test.subject.value
                    && let Some(index) = self.local_position(name)?
                {
                    self.proof.names[index].1 = Value {
                        available: self.proof.names[index].1.available,
                        ..subject
                    };
                }
                if narrowing && !place && let Some(owner) = subject.owner {
                    self.charge_flow()?;
                    self.obligate(owner);
                }
                if place
                    && !narrowing
                    && let Expr::Name(name) = &test.subject.value
                    && linear
                    && let Some(index) = self.local_position(name)?
                {
                    self.proof.names[index].1.available = false;
                }
                pending.push(EvalTask::Scope(saved, hidden));
            }
            pending.push(EvalTask::Expression(&case.then_branch, expected, false));
        } else {
            if !place && let Some(owner) = subject.owner {
                self.charge_flow()?;
                self.obligate(owner);
            }
            pending.push(EvalTask::Expression(&case.else_branch, expected, false));
        }
        Ok(())
    }
}
