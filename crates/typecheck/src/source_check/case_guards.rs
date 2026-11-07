//! A one-armed case that returns: `when subject is Pattern then <block ending in return>` at the
//! start of a function's body, with the rest of the body after it.
//!
//! The subject's member is a replay choice, exactly as in a full `when`: in the world where it is
//! the pattern's variant the block runs and the path ends; in every other world the statement is
//! passed over and the rest of the body runs. So a call result tested for a failure variant costs
//! one world per variant, and `n` such tests in a row make `n + 1` paths, not `2^n`.
//!
//! A named subject is read in place, not moved, so in the worlds where the pattern does not match
//! it is still there, now known to be one of the other variants: flow typing on a place. An
//! unnamed subject (a call result) is a temporary, and what the pattern does not take must be
//! something that can be dropped or the function fails to end it. In the matching world a
//! pattern consumes the subject, as in a full case.
use super::*;
use aggregates::EvalTask;

impl<'a> Checker<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn case_guard_tasks(
        &mut self,
        when: &'a WhenIsStatement,
        span: SpanId,
        expected: Option<SourceType>,
        base: usize,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let MatchPattern::Variant(pattern) = &when.pattern.value else {
            return Err(SourceCheckError::Unsupported);
        };
        let place = assign::split_place(&when.subject).is_some();
        if !visited {
            self.keyword(when.when_keyword, "when")?;
            self.keyword(when.is_keyword, "is")?;
            self.keyword(when.then_keyword, "then")?;
            self.child(span, when.subject.span_id)?;
            self.child(span, when.pattern.span_id)?;
            self.child(span, when.then_block.span_id)?;
            self.charge(4)?;
            pending.push(EvalTask::CaseGuard(when, span, expected, base, true));
            if !place {
                pending.push(EvalTask::Expression(&when.subject, None, false));
            }
            return Ok(());
        }
        let subject = if place {
            self.project_place(&when.subject)?
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
        let schema = self.pattern_schema(pattern, when.pattern.span_id, &variants)?;
        self.charge(variants.len())?;
        let named = variants.iter().any(|variant| {
            matches!(variant, SourceType::Aggregate { id }
                if self.module.instances[*id as usize].schema() == schema)
        });
        if !named {
            return Err(SourceCheckError::Type);
        }
        if schema != active {
            // Not this member: the statement is passed over. A named subject stays available,
            // known to be what it is in this world. An unnamed one is a temporary nobody holds,
            // so it is discarded, which a value that must be consumed cannot be.
            if !place && let Some(owner) = subject.owner {
                self.charge_flow()?;
                self.obligate(owner);
            }
            return Ok(());
        }
        // The claims of this member hold in the world where it is the one.
        self.apply_claims(subject)?;
        // Whether the pattern consumes the subject is read before it does.
        let linear = self.holds_linear(subject);
        // This member: bind its fields for the block. A block that returns ends the path, so
        // nothing after it runs; one that falls through goes on to the rest of the body.
        let returns = guards::returns(&when.then_block.value);
        if returns {
            pending.truncate(base);
            if self.proof.loop_depth > 0 {
                pending.push(EvalTask::Returned);
            }
        }
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
        self.bind_pattern(pattern, when.pattern.span_id, subject)?;
        let narrowing = self.narrows(pattern, schema);
        let unnarrow = match &when.subject.value {
            Expr::Name(name) if narrowing && !returns => {
                let index = self.local_position(name)?.ok_or(SourceCheckError::Malformed)?;
                Some((name.as_str(), self.proof.names[index].1))
            }
            _ => None,
        };
        // In this world a named subject is the member: the name now holds it so typed.
        if narrowing
            && let Expr::Name(name) = &when.subject.value
            && let Some(index) = self.local_position(name)?
        {
            self.proof.names[index].1 = Value {
                available: self.proof.names[index].1.available,
                ..subject
            };
        }
        // A narrowing test leaves an unnamed subject a temporary that must still be consumed.
        if narrowing && !place && let Some(owner) = subject.owner {
            self.charge_flow()?;
            self.obligate(owner);
        }
        // A pattern consumes a linear subject: the block may not use the name again. A
        // narrowing test does not: the name is the same value, now known to be the member.
        if place
            && !narrowing
            && let Expr::Name(name) = &when.subject.value
            && linear
            && let Some(index) = self.local_position(name)?
        {
            self.proof.names[index].1.available = false;
        }
        if let Some((name, before)) = unnarrow {
            pending.push(EvalTask::Unnarrow(name, before));
        }
        pending.push(EvalTask::Scope(saved, hidden));
        self.statement_tasks(
            when.then_block.span_id,
            &when.then_block.value.statements,
            if returns { expected } else { None },
            if returns { base } else { usize::MAX },
            pending,
        )
    }
}
