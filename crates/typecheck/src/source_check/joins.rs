//! A `when` statement whose branches fall through: `when c then <block> [else <block>]` with no
//! `return`, followed by more statements.
//!
//! The statement is checked once, not once per branch: the then-block runs under the comparison,
//! the state is put back, the else-block runs under its negation, and the two ends are joined.
//! That keeps a sequence of `k` such statements at `k` joins instead of `2^k` worlds. A branch that
//! itself makes a replay choice still multiplies with the other branch's choices; isolating those is a
//! later step.
//!
//! What the join keeps: a local both branches left alone, and for a local either one assigned a
//! fresh number spanning both values. What it drops: the comparison and every fact the branches
//! added. What it refuses (`Unsupported`): an owner whose state differs between the ends, since
//! there is no maybe-moved state, and an assigned local that is not a plain integer.
use super::*;
use aggregates::EvalTask;
use sums::ValidationState;

/// The state a join carries from the branch point to its end.
pub(super) struct JoinFrame<'a> {
    when: &'a WhenStatement,
    before: ValidationState<'a>,
    assignable: Vec<&'a str>,
    /// The state at the end of the then-block, once it has run.
    then_end: Option<ValidationState<'a>>,
    relation: guards::Relation,
    left: Value,
    right: guards::Operand,
    /// Whether the else path is possible, so whether there is anything to join with.
    fails: bool,
}

impl<'a> Checker<'a> {
    pub(super) fn join_tasks(
        &mut self,
        when: &'a WhenStatement,
        span: SpanId,
        expected: Option<SourceType>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let (_, leaves) =
            guards::conditions(&when.condition).ok_or(SourceCheckError::Unsupported)?;
        let [leaf] = leaves[..] else {
            return Err(SourceCheckError::Unsupported);
        };
        let relation = leaf.relation;
        if !visited {
            let blocks = std::iter::once(&when.then_block)
                .chain(when.else_branch.iter().map(|branch| &branch.block));
            for block in blocks {
                // A branch that returns is a guard; here both must fall through.
                if block
                    .value
                    .statements
                    .iter()
                    .any(|statement| matches!(statement.value, Statement::Return(_)))
                {
                    return Err(SourceCheckError::Unsupported);
                }
            }
            self.keyword(when.when_keyword, "when")?;
            self.keyword(when.then_keyword, "then")?;
            self.child(span, when.condition.span_id)?;
            self.child(span, when.then_block.span_id)?;
            if let Some(branch) = &when.else_branch {
                self.keyword(branch.else_keyword, "else")?;
                self.child(span, branch.else_keyword)?;
                self.child(span, branch.block.span_id)?;
            }
            self.condition_syntax(&when.condition)?;
            self.charge(3)?;
            pending.push(EvalTask::Join(when, span, expected, true));
            pending.push(leaf.right.task());
            pending.push(EvalTask::Expression(leaf.left, None, false));
            return Ok(());
        }
        let right = self.pop_operand(relation, values)?;
        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
        // An unrestricted domain compares as the number it holds.
        let left = self.condition_number(left)?;
        self.scalar(left)?;
        let left = self.ensure_identity(left)?;
        let holds = relation.possible(left, right);
        let fails = relation.negate().possible(left, right);
        let before = self.validation_state()?;
        let frame = JoinFrame {
            when,
            before,
            assignable: self.proof.assignable.clone(),
            then_end: None,
            relation,
            left,
            right,
            fails,
        };
        if holds {
            self.assume_relation(relation, left, right)?;
            pending.push(EvalTask::JoinThen(Box::new(frame)));
            self.statement_tasks(
                when.then_block.span_id,
                &when.then_block.value.statements,
                None,
                usize::MAX,
                pending,
            )?;
            Ok(())
        } else {
            self.join_else_phase(frame, pending)
        }
    }

    /// The then-block has run. If the else path is possible, run it from the branch point.
    pub(super) fn join_then(
        &mut self,
        mut frame: JoinFrame<'a>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        if !frame.fails {
            // The then path is the only one: it continues as it is, keeping what it learned.
            self.leave_scope(&frame);
            return Ok(());
        }
        frame.then_end = Some(self.validation_state()?);
        self.join_else_phase(frame, pending)
    }

    fn join_else_phase(
        &mut self,
        frame: JoinFrame<'a>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.proof.restore_validation(frame.before.clone());
        self.proof.assignable = frame.assignable.clone();
        self.assume_relation(frame.relation.negate(), frame.left, frame.right)?;
        let when = frame.when;
        pending.push(EvalTask::JoinElse(Box::new(frame)));
        if let Some(branch) = &when.else_branch {
            self.statement_tasks(
                branch.block.span_id,
                &branch.block.value.statements,
                None,
                usize::MAX,
                pending,
            )?;
        }
        Ok(())
    }

    /// The else-block has run (or was empty): join its end with the then-block's.
    pub(super) fn join_else(&mut self, mut frame: JoinFrame<'a>) -> Result<()> {
        let Some(then_end) = frame.then_end.take() else {
            // Only the else path was possible.
            self.leave_scope(&frame);
            return Ok(());
        };
        let count = frame.before.names.len();
        let owners = frame.before.owners.len();
        self.charge(count + owners)?;
        // Owners must be in the same state at both ends: there is no maybe-moved state.
        for index in 0..owners {
            let (a, b) = (&then_end.owners[index], &self.proof.owners[index]);
            let same = a.live == b.live
                && a.moved == b.moved
                && a.slots == b.slots
                && a.materialized == b.materialized
                && a.drained == b.drained
                && a.children.len() == b.children.len();
            if !same {
                return Err(SourceCheckError::Unsupported);
            }
        }
        // An owner created inside a branch must be gone by its end.
        let created = then_end.owners[owners..]
            .iter()
            .chain(&self.proof.owners[owners..])
            .any(|owner| owner.live);
        if created {
            return Err(SourceCheckError::Unconsumed);
        }
        let else_names = self.proof.names[..count].to_vec();
        let mut merged = Vec::with_capacity(count);
        let mut spans = Vec::new();
        for index in 0..count {
            let (name, original) = frame.before.names[index];
            let (a, b) = (then_end.names[index].1, else_names[index].1);
            // A literal has no identity, so a changed number is told apart by its bounds.
            let untouched = |value: Value| {
                value.identity == original.identity
                    && value.lower == original.lower
                    && value.upper == original.upper
                    && value.owner == original.owner
                    && value.borrow == original.borrow
                    && value.available == original.available
            };
            if untouched(a) && untouched(b) {
                merged.push((name, original));
                continue;
            }
            if !matches!(original.ty, SourceType::Integer { .. })
                || !matches!(a.ty, SourceType::Integer { .. })
                || !matches!(b.ty, SourceType::Integer { .. })
            {
                return Err(SourceCheckError::Unsupported);
            }
            spans.push((index, a, b));
            merged.push((name, original));
        }
        // Back to the branch point, minus what the comparison taught, plus the joined numbers.
        let next_identity = then_end.next_identity.max(self.proof.next_identity);
        let before = frame.before;
        self.proof.restore_validation(before);
        self.proof.next_identity = next_identity;
        self.proof.assignable = frame.assignable;
        self.proof.names = merged;
        for (index, a, b) in spans {
            let lower = a.lower.min(b.lower);
            let upper = a.upper.max(b.upper);
            let fresh = self.fresh_value(SourceType::Integer { lower, upper })?;
            let original = self.proof.names[index].1;
            self.proof.names[index].1 = Value {
                integer_domain: original.integer_domain,
                available: true,
                affine: None,
                ..fresh
            };
        }
        Ok(())
    }

    /// Drop what the branches declared: locals bound inside them end with the statement.
    fn leave_scope(&mut self, frame: &JoinFrame<'a>) {
        self.proof.names.truncate(frame.before.names.len());
        self.proof.assignable.retain(|name| frame.assignable.contains(name));
    }
}
