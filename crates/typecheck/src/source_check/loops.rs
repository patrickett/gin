//! Finite consuming iteration keeps pending positions in the original structural root.
//! Frames live only on the verification stack; published summaries retain no loop syntax.
use super::*;
use aggregates::EvalTask;
use contracts::Constraint;

/// A finite-range loop whose bounds are being evaluated.
pub(super) struct RangeFrame<'a> {
    binder: &'a str,
    body: &'a Expression,
    /// The body can branch (a conditional, a case, or an inner loop).
    branches: bool,
    /// `where P`: what holds at the head of every pass, written by the user and never inferred.
    invariant: Option<&'a ast::LoopInvariant>,
}

/// How many passes a loop with a known count may be unrolled exactly. A body that branches
/// is held to fewer, since each undecided condition doubles the worlds checked per pass.
const UNROLL_STRAIGHT: u32 = 64;
const UNROLL_BRANCHING: u32 = 8;

/// The same for a count known only to lie below a bound. Every pass past the first may be
/// where the loop stops, and each stopping point is a world that carries on with the rest of
/// the function, so these limits are lower.
const GUARDED_STRAIGHT: u32 = 16;
const GUARDED_BRANCHING: u32 = 4;

pub(super) struct Frame<'a> {
    iterable: Value,
    binder: &'a str,
    body: &'a Expression,
    next: u32,
    count: u32,
}

struct FramedOwner {
    id: usize,
    live: bool,
    moved: Vec<u32>,
    drained: bool,
}

#[derive(Default)]
struct LoopScan<'a> {
    branches: bool,
    modified: Vec<&'a str>,
    /// A return or a nested loop can end the path before the backedge.
    ends_path: bool,
}

pub(super) struct Iteration<'a> {
    frame: Box<Frame<'a>>,
    names: Vec<(&'a str, Value)>,
    inventory: Vec<FramedOwner>,
    /// Existing owners in this element's subtree, not unrelated captured obligations.
    current: Vec<bool>,
    owners_before: usize,
    values_before: usize,
}

impl<'a> Checker<'a> {
    pub(super) fn loop_tasks(
        &mut self,
        expression: &'a Expression,
        loop_expression: &'a LoopExpression,
        expected: Option<SourceType>,
        visited: bool,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        if expected.is_some_and(|ty| ty != SourceType::Unit) {
            return Err(SourceCheckError::Type);
        }
        if let LoopExpression::While { .. } = loop_expression {
            self.while_loop(expression, loop_expression)?;
            values.push(self.proof.returned_value.take().unwrap_or_else(Value::unit));
            return Ok(());
        }
        let LoopExpression::For {
            binder,
            iterable,
            body,
            invariant,
            ..
        } = loop_expression
        else {
            return Err(SourceCheckError::Unsupported);
        };
        if !visited {
            self.loop_syntax(expression, loop_expression)?;
            let exits = matches!(&iterable.value, Expr::Range { .. });
            let branches = self.loop_exit_free(body, exits)?;
            if let Expr::Range {
                start,
                to_keyword,
                end,
            } = &iterable.value
            {
                // A count that is not known cannot be unrolled: the body is checked once.
                self.span(iterable.span_id)?;
                self.child(iterable.span_id, start.span_id)?;
                self.child(iterable.span_id, end.span_id)?;
                self.keyword(*to_keyword, "to")?;
                self.charge(3)?;
                let binder = self.name(binder)?;
                // With an invariant the bounds are related to their operands (`k - 1` to `k`), so
                // what the loop learns about the counter reaches the numbers it was written from.
                self.proof.relate_always = invariant.is_some();
                pending.push(EvalTask::RangeLoop(Box::new(RangeFrame {
                    binder,
                    body,
                    branches,
                    invariant: invariant.as_ref(),
                })));
                pending.push(EvalTask::Expression(end, None, false));
                pending.push(EvalTask::Expression(start, None, false));
                return Ok(());
            }
            self.charge(2)?;
            pending.push(EvalTask::Expression(expression, expected, true));
            pending.push(EvalTask::Expression(iterable, None, false));
            return Ok(());
        }
        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
        if value.ty == SourceType::Unit {
            self.loop_empty_body(body)?;
            values.push(Value::unit());
            return Ok(());
        }
        if matches!(value.ty, SourceType::Array { .. }) {
            let binder = self.name(binder)?;
            self.drain_loop(binder, body, value)?;
            values.push(Value::unit());
            return Ok(());
        }
        if !matches!(value.ty, SourceType::Sequence { .. }) {
            return Err(SourceCheckError::Unsupported);
        }
        self.array_complete(value)?;
        let count = self.position_count(value.ty)?;
        let binder = self.name(binder)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.obligate(owner);
        pending.push(EvalTask::LoopNext(Box::new(Frame {
            iterable: value,
            binder,
            body,
            next: 0,
            count,
        })));
        Ok(())
    }

    /// Check a loop body once for an iteration count that is not known.
    ///
    /// The body is checked against the weakest state a previous iteration could leave:
    /// everything it might change is forgotten first, and forgotten again afterwards, since
    /// any number of iterations (including none) may have run. Nothing it assumes survives it.
    /// `while left <comparison> right do body`, checked once regardless of iteration count.
    ///
    /// At the loop head the body may have run any number of times, so everything it could change
    /// is forgotten first (down to the declared domains, as `forget_loop_state` does for range
    /// loops). The condition is read there, the body is checked once under it, and one pass must
    /// leave every owner as it found it. After the loop the head state holds again, with the
    /// condition false. Nothing is inferred: what a pass establishes is lost unless the
    /// declared domain says it.
    ///
    /// A condition that cannot hold at the head cannot hold on entry either, because the head
    /// state spans the entry state, so the loop never runs and the state is the entry state.
    fn while_loop(
        &mut self,
        expression: &'a Expression,
        loop_expression: &'a LoopExpression,
    ) -> Result<()> {
        let LoopExpression::While {
            while_keyword,
            condition,
            invariant,
            do_keyword,
            body,
        } = loop_expression
        else {
            return Err(SourceCheckError::Unsupported);
        };
        let (connective, leaves) =
            guards::conditions(condition).ok_or(SourceCheckError::Unsupported)?;
        self.keyword(*while_keyword, "while")?;
        self.keyword(*do_keyword, "do")?;
        self.child(expression.span_id, *while_keyword)?;
        self.child(expression.span_id, condition.span_id)?;
        self.child(expression.span_id, *do_keyword)?;
        self.child(expression.span_id, body.span_id)?;
        self.condition_syntax(condition)?;
        self.invariant_syntax(expression, invariant.as_ref())?;
        let names: Vec<&str> = self.proof.names.iter().map(|(name, _)| *name).collect();
        let invariant = self.invariant_relations(invariant.as_ref(), &names)?;
        let scan = self.loop_scan(body, true)?;
        let mut modified = scan.modified;
        modified.extend(self.loop_scan(condition, true)?.modified);
        self.charge(8 + modified.len())?;

        // The invariant holds on entry, before anything is forgotten.
        self.prove_invariant(&invariant)?;
        let entry = self.validation_state()?;
        let entry_assignable = self.proof.assignable.clone();

        // What a call leaves as it found it is kept across the loop, as for a range loop.
        let entry_leaves = self.frame_leaves(&modified);
        let kept = if entry_leaves.is_empty() {
            Vec::new()
        } else {
            self.unchanged_leaves_with(body.span_id, &modified, &entry_leaves, |this| {
                this.assume_invariant(&invariant)?;
                if connective == guards::Connective::And {
                    for leaf in &leaves {
                        let left = this.evaluate(leaf.left, None)?;
                        this.scalar(left)?;
                        let left = this.ensure_identity(left)?;
                        let right = this.evaluate_operand(leaf.right, leaf.relation)?;
                        this.assume_relation(leaf.relation, left, right)?;
                    }
                }
                this.loop_body(body)
            })?
        };
        self.forget_named(Some(&modified))?;
        // At the head of a pass the invariant holds of whatever the body changed.
        self.assume_invariant(&invariant)?;
        let head_leaves = self.assume_frame(&kept)?;
        let mut tests = Vec::with_capacity(leaves.len());
        for leaf in &leaves {
            let left = self.evaluate(leaf.left, None)?;
            self.scalar(left)?;
            let left = self.ensure_identity(left)?;
            let right = self.evaluate_operand(leaf.right, leaf.relation)?;
            tests.push((left, right, leaf.relation));
        }
        let holds: Vec<usize> = (0..tests.len())
            .filter(|&index| tests[index].2.possible(tests[index].0, tests[index].1))
            .collect();
        // TODO: a body that never runs is never checked, so a mistake in it goes unreported.
        let enters = match connective {
            guards::Connective::And => holds.len() == tests.len(),
            guards::Connective::Or => !holds.is_empty(),
        };
        if !enters {
            self.proof.restore_validation(entry);
            self.proof.assignable = entry_assignable;
            return Ok(());
        }
        let fails: Vec<usize> = (0..tests.len())
            .filter(|&index| {
                tests[index]
                    .2
                    .negate()
                    .possible(tests[index].0, tests[index].1)
            })
            .collect();
        let exits = match connective {
            guards::Connective::And => !fails.is_empty(),
            guards::Connective::Or => fails.len() == tests.len(),
        };
        // A return in the entered body must not erase a world where the condition fails.
        // Choose before running the body so its owners, facts and return evidence stay there.
        if scan.ends_path && exits && self.choose(2)? == 1 {
            return self.while_exit(connective, &tests, &fails);
        }
        // The head: what holds before the condition is assumed, and what the loop ends in.
        // The operands already have their identities, so facts about them survive its restore.
        let owners_before = self.proof.owners.len();
        let owners: Vec<(bool, usize, bool)> = self
            .proof
            .owners
            .iter()
            .map(|owner| (owner.live, owner.moved.len(), owner.drained))
            .collect();
        self.charge(owners.len())?;
        let head = self.validation_state()?;
        let head_assignable = self.proof.assignable.clone();
        let head_names = head.names.clone();
        match connective {
            // Every comparison holds inside the body ...
            guards::Connective::And => {
                for &(left, right, relation) in &tests {
                    self.assume_relation(relation, left, right)?;
                }
            }
            // ... or, for `or`, one of them does: the body is checked once for each.
            guards::Connective::Or => {
                let picked = holds[self.choose(holds.len())?];
                let (left, right, relation) = tests[picked];
                self.assume_relation(relation, left, right)?;
            }
        }
        let result = self.loop_body(body)?;
        if self.proof.returned {
            // The path ended in the body with a `return`; nothing after the loop runs on it.
            self.proof.returned_value = Some(result);
            return Ok(());
        }
        if let Some(owner) = result.owner {
            self.obligate(owner);
        }
        self.loop_pass_obligations(&head_names, &owners, owners_before)?;
        // The pass must leave the invariant true for the next one, and the kept numbers as it found them.
        self.prove_invariant(&invariant)?;
        self.prove_frame(&head_leaves)?;

        // The loop ends when the condition fails, at a head state like the one it started from.
        // Ownership was checked above; the numbers are exactly the head's.
        self.proof.restore_validation(head);
        self.proof.assignable = head_assignable;
        self.while_exit(connective, &tests, &fails)
    }

    /// Continue only in a world where the head condition fails.
    fn while_exit(
        &mut self,
        connective: guards::Connective,
        tests: &[(Value, guards::Operand, guards::Relation)],
        fails: &[usize],
    ) -> Result<()> {
        match connective {
            // `or` ends when every comparison has failed ...
            guards::Connective::Or if fails.len() == tests.len() => {
                for &(left, right, relation) in tests {
                    self.assume_relation(relation.negate(), left, right)?;
                }
            }
            // ... `and` when any one has.
            guards::Connective::And if !fails.is_empty() => {
                let picked = fails[self.choose(fails.len())?];
                let (left, right, relation) = tests[picked];
                self.assume_relation(relation.negate(), left, right)?;
            }
            // The condition always holds: the loop does not end, so nothing after it runs. The
            // path stops here as at a `return`; what it gives is never seen, so it is a value of
            // whatever the function gives.
            _ => {
                let given = match self.proof.result_expected {
                    Some(ty) if ty != SourceType::Unit => self.fresh_value(ty)?,
                    _ => Value::unit(),
                };
                self.proof.returned = true;
                self.proof.returned_value = Some(given);
            }
        }
        Ok(())
    }

    /// The invariant's syntax joins the loop's: its keyword and condition are children of it.
    pub(super) fn invariant_syntax(
        &mut self,
        loop_expression: &'a Expression,
        invariant: Option<&'a ast::LoopInvariant>,
    ) -> Result<()> {
        if let Some(invariant) = invariant {
            self.keyword(invariant.where_keyword, "where")?;
            self.child(loop_expression.span_id, invariant.where_keyword)?;
            self.child(loop_expression.span_id, invariant.condition.span_id)?;
        }
        Ok(())
    }

    /// The name now stands for `value`: a loop's counter for the next pass.
    fn rebind(&mut self, name: &'a str, value: Value) -> Result<()> {
        let position = self
            .proof
            .names
            .iter()
            .rposition(|(other, _)| *other == name)
            .ok_or(SourceCheckError::UnknownName)?;
        self.proof.names[position].1 = value;
        Ok(())
    }

    /// The relations of an invariant whose spans a loop's own syntax check has already placed,
    /// compiled as the contract terms a parameter's `and` claim is: a place such as
    /// `self.length` unfolds through the type's accessors to the field it reads, so reading it
    /// before and after a pass talks about one number, and `first + count` is related to its
    /// operands. `names` are the locals the terms may mention, in the order they are bound.
    fn invariant_relations(
        &mut self,
        invariant: Option<&'a ast::LoopInvariant>,
        names: &[&str],
    ) -> Result<Vec<Constraint>> {
        let Some(invariant) = invariant else {
            return Ok(Vec::new());
        };
        let mut constraints = Vec::new();
        let mut pending = vec![&invariant.condition];
        while let Some(expression) = pending.pop() {
            self.charge(2)?;
            match &expression.value {
                Expr::Group(inner) => {
                    self.group_syntax(expression, inner)?;
                    pending.push(inner);
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } if operator.value == BinaryOperator::And => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.keyword(operator.span_id, "and")?;
                    pending.extend([right.as_ref(), left.as_ref()]);
                }
                Expr::Binary { operator, .. }
                    if !matches!(
                        operator.value,
                        BinaryOperator::Equal
                            | BinaryOperator::Less
                            | BinaryOperator::LessOrEqual
                            | BinaryOperator::Greater
                            | BinaryOperator::GreaterOrEqual
                    ) =>
                {
                    return Err(SourceCheckError::Unsupported);
                }
                Expr::Binary { .. } => {
                    self.relation_constraint(expression, names, &mut constraints)?
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(constraints)
    }

    /// The invariant must be shown at this point (entry, and the end of a pass).
    pub(super) fn prove_invariant(&mut self, constraints: &[Constraint]) -> Result<()> {
        if constraints.is_empty() {
            return Ok(());
        }
        let mut values: Vec<Value> = self.proof.names.iter().map(|(_, value)| *value).collect();
        self.constraints(constraints, &mut values, false)
            .map_err(|error| match error {
                SourceCheckError::Unproved => {
                    self.explain(error, "the loop invariant is not shown here".to_owned())
                }
                other => other,
            })
    }

    /// The invariant is taken to hold (the head of a pass, and the exit).
    pub(super) fn assume_invariant(&mut self, constraints: &[Constraint]) -> Result<()> {
        if constraints.is_empty() {
            return Ok(());
        }
        let mut values: Vec<Value> = self.proof.names.iter().map(|(_, value)| *value).collect();
        self.constraints(constraints, &mut values, true)
    }

    /// One pass of a loop body, evaluated one loop deeper: a `return` inside it ends the path
    /// and leaves `returned` set for the loops and evaluations around it to unwind.
    pub(super) fn loop_body(&mut self, body: &'a Expression) -> Result<Value> {
        self.proof.loop_depth += 1;
        let result = self.evaluate(body, None);
        self.proof.loop_depth -= 1;
        result
    }

    /// One pass must leave every outer owner as it found it, or the next pass would use a
    /// value already moved, and must end everything it made itself.
    pub(super) fn range_loop(
        &mut self,
        frame: &RangeFrame<'a>,
        start: Value,
        end: Value,
    ) -> Result<()> {
        self.proof.relate_always = false;
        self.scalar(start)?;
        self.scalar(end)?;
        // A range that is certainly empty never runs its body.
        if start.lower > end.upper {
            let counter = SourceType::Integer {
                lower: end.upper,
                upper: start.lower,
            };
            return self.check_unreachable(
                frame.body.span_id,
                "this loop never runs: its range is empty",
                |this| {
                    let counter = this.fresh_value(counter)?;
                    this.insert(frame.binder, counter)?;
                    this.evaluate(frame.body, None)
                },
            );
        }
        // With both bounds known the count is known: run the passes one after another, so
        // everything the body establishes carries from one pass to the next exactly.
        let limit = if frame.branches {
            UNROLL_BRANCHING
        } else {
            UNROLL_STRAIGHT
        };
        // A written invariant is how a loop's total effect is learned: the body is checked once.
        let written = frame.invariant.is_some();
        let known = start.lower == start.upper && end.lower == end.upper;
        if !written && known && end.upper - start.lower < I256::from(limit) {
            return self.unrolled_loop(frame, start.lower, end.upper);
        }
        // With only the bounds' ranges known, the count is at most `end.upper - start.lower + 1`,
        // so a small enough one is still unrolled, each pass behind its own guard.
        let guarded = if frame.branches {
            GUARDED_BRANCHING
        } else {
            GUARDED_STRAIGHT
        };
        if !written && end.upper - start.lower < I256::from(guarded) {
            return self.guarded_loop(frame, start, end);
        }
        let start = self.ensure_identity(start)?;
        let end = self.ensure_identity(end)?;
        let mut leaves = Vec::new();
        if written {
            // On entry the counter is the start of the range.
            let before = self.proof.names.len();
            self.insert(frame.binder, start)?;
            let names: Vec<&str> = self.proof.names.iter().map(|(name, _)| *name).collect();
            leaves = self.invariant_relations(frame.invariant, &names)?;
            self.prove_invariant(&leaves)?;
            self.proof.names.truncate(before);
        }
        // Only what the body can change is forgotten: a local it does not touch keeps every fact
        // the enclosing conditions gave it.
        let scan = self.loop_scan(frame.body, true)?;
        // The entered pass may return, but an empty range still reaches the following code
        // with its original owners and facts. Keep that world outside the body assumptions.
        if scan.ends_path
            && Self::comparison_possible(BinaryOperator::Greater, start, end)
            && self.choose(2)? == 1
        {
            self.assume(BinaryOperator::Greater, start, end)?;
            return Ok(());
        }
        let modified = scan.modified;
        // What a call leaves as it found it (`capacity :: capacity`) is kept across the loop: the
        // numbers of a record the body changes that every pass hands back unchanged.
        let entry_leaves = self.frame_leaves(&modified);
        let kept = if entry_leaves.is_empty() {
            Vec::new()
        } else {
            self.unchanged_leaves(frame, start, end, &modified, &leaves, &entry_leaves)?
        };
        self.forget_named(Some(&modified))?;
        let head_leaves = self.assume_frame(&kept)?;
        let names = self.proof.names.clone();
        let facts = self.proof.facts.clone();
        let scalars = self.proof.scalars.clone();
        let guards = self.proof.integer_guards.clone();
        let assignable = self.proof.assignable.clone();
        let owners_before = self.proof.owners.len();
        let entry: Vec<(bool, usize, bool)> = self
            .proof
            .owners
            .iter()
            .map(|owner| (owner.live, owner.moved.len(), owner.drained))
            .collect();
        self.charge(names.len() + entry.len())?;

        // The counter lies between the bounds, and each fact says so for the body only.
        let counter = self.fresh_value(SourceType::Integer {
            lower: start.lower,
            upper: end.upper,
        })?;
        self.assume(BinaryOperator::GreaterOrEqual, counter, start)?;
        self.assume(BinaryOperator::LessOrEqual, counter, end)?;
        self.insert(frame.binder, counter)?;
        // At the head of a pass the invariant holds of whatever the body changed.
        self.assume_invariant(&leaves)?;
        let result = self.loop_body(frame.body)?;
        if self.proof.returned {
            self.proof.returned_value = Some(result);
            return Ok(());
        }
        if let Some(owner) = result.owner {
            self.obligate(owner);
        }

        self.loop_pass_obligations(&names, &entry, owners_before)?;
        // Each pass hands back what it was given: the kept numbers are the ones at the head.
        self.prove_frame(&head_leaves)?;
        if written {
            // The next pass has the counter one higher, and the invariant must hold for it.
            let one = Value::integer(I256::ONE, I256::ONE);
            self.proof.relate_always = true;
            let next = self.arithmetic(CheckedArithmetic::Add, counter, one);
            self.proof.relate_always = false;
            let next = next?;
            let next = self.ensure_identity(next)?;
            self.rebind(frame.binder, next)?;
            self.prove_invariant(&leaves)?;
        }

        self.proof.names = names;
        self.proof.facts = facts;
        self.proof.scalars = scalars;
        self.proof.integer_guards = guards;
        self.proof.assignable = assignable;
        self.forget_named(Some(&modified))?;
        self.assume_frame(&kept)?;
        if written {
            // The loop ended with the counter just past the end, when it ran at all (`start` at
            // most `end + 1`); the invariant holds of that counter. When that is not shown,
            // only the declared domains survive.
            let one = Value::integer(I256::ONE, I256::ONE);
            self.proof.relate_always = true;
            let past = self.arithmetic(CheckedArithmetic::Add, end, one);
            self.proof.relate_always = false;
            let past = past?;
            let past = self.ensure_identity(past)?;
            if self
                .compare(BinaryOperator::LessOrEqual, start, past)
                .is_ok()
            {
                let before = self.proof.names.len();
                self.insert(frame.binder, past)?;
                self.assume_invariant(&leaves)?;
                self.proof.names.truncate(before);
            }
        }
        Ok(())
    }

    /// One pass must leave every outer owner as it found it, or the next pass would use a
    /// value already moved, and must end everything it made itself.
    pub(super) fn loop_pass_obligations(
        &mut self,
        names: &[(&'a str, Value)],
        entry: &[(bool, usize, bool)],
        owners_before: usize,
    ) -> Result<()> {
        // One pass must leave the outer owners as it found them...
        for (name, before) in names {
            let now = self
                .proof
                .names
                .iter()
                .find(|(other, _)| other == name)
                .map(|(_, value)| *value)
                .ok_or(SourceCheckError::Malformed)?;
            if before.owner.is_some() && before.available && !now.available {
                return Err(SourceCheckError::UseAfterMove);
            }
        }
        self.charge(entry.len())?;
        for (index, (live, moved, drained)) in entry.iter().enumerate() {
            let owner = &self.proof.owners[index];
            if owner.live != *live || owner.moved.len() != *moved || owner.drained != *drained {
                return Err(if *live && !owner.live {
                    SourceCheckError::UseAfterMove
                } else {
                    SourceCheckError::Unconsumed
                });
            }
        }
        // ...and end whatever it made itself.
        if self.proof.owners[owners_before..]
            .iter()
            .any(|owner| owner.live)
        {
            return Err(SourceCheckError::Unconsumed);
        }
        Ok(())
    }

    /// A loop of a known count, one pass at a time. The counter is the exact number of that
    /// pass, so nothing is forgotten: each pass sees what the one before established. A pass
    /// may consume an outer owner (a second pass then fails as a use after move), but must
    /// end every resource it made itself.
    fn unrolled_loop(&mut self, frame: &RangeFrame<'a>, first: I256, last: I256) -> Result<()> {
        let names = self.proof.names.len();
        let mut number = first;
        loop {
            self.charge(2)?;
            let owners_before = self.proof.owners.len();
            let counter = self.fresh_value(SourceType::Integer {
                lower: number,
                upper: number,
            })?;
            self.insert(frame.binder, counter)?;
            let result = self.loop_body(frame.body)?;
            if self.proof.returned {
                self.proof.returned_value = Some(result);
                return Ok(());
            }
            if let Some(owner) = result.owner {
                self.obligate(owner);
            }
            if self.proof.owners[owners_before..]
                .iter()
                .any(|owner| owner.live)
            {
                return Err(SourceCheckError::Unconsumed);
            }
            // Names the pass bound are out of scope; outer names keep what it did to them.
            self.proof.names.truncate(names);
            if number == last {
                return Ok(());
            }
            number = number
                .checked_add(I256::ONE)
                .ok_or(SourceCheckError::Overflow)?;
        }
    }

    /// A loop whose count lies in a small range: the passes one after another, each one only
    /// when the loop has not stopped yet. Before pass `k` the checker chooses a world in
    /// which the loop stops there or one in which it runs, so each stopping point keeps
    /// exactly what the passes before it established. Pass `k` has the counter `start + k`,
    /// known as far as the ranges of the bounds say, and between the bounds for the body.
    fn guarded_loop(&mut self, frame: &RangeFrame<'a>, start: Value, end: Value) -> Result<()> {
        let start = self.ensure_identity(start)?;
        let end = self.ensure_identity(end)?;
        let names = self.proof.names.len();
        let zero = I256::from(0);
        let mut offset = zero;
        loop {
            self.charge(2)?;
            let lower = start
                .lower
                .checked_add(offset)
                .ok_or(SourceCheckError::Overflow)?;
            // No count reaches this pass: the loop has certainly stopped.
            if lower > end.upper {
                return Ok(());
            }
            let upper = start
                .upper
                .checked_add(offset)
                .ok_or(SourceCheckError::Overflow)?;
            // The loop may stop before this pass unless its bounds say it cannot.
            if upper > end.lower && self.choose(2)? == 1 {
                return Ok(());
            }
            let owners_before = self.proof.owners.len();
            let counter = self.fresh_value(SourceType::Integer {
                lower,
                upper: upper.min(end.upper),
            })?;
            self.assume(BinaryOperator::GreaterOrEqual, counter, start)?;
            self.assume(BinaryOperator::LessOrEqual, counter, end)?;
            self.insert(frame.binder, counter)?;
            let result = self.loop_body(frame.body)?;
            if self.proof.returned {
                self.proof.returned_value = Some(result);
                return Ok(());
            }
            if let Some(owner) = result.owner {
                self.obligate(owner);
            }
            if self.proof.owners[owners_before..]
                .iter()
                .any(|owner| owner.live)
            {
                return Err(SourceCheckError::Unconsumed);
            }
            self.proof.names.truncate(names);
            offset = offset
                .checked_add(I256::ONE)
                .ok_or(SourceCheckError::Overflow)?;
        }
    }

    /// Check code that can never run, in a world nothing keeps: a mistake in it is still a
    /// mistake, and it is reported as unreachable. What it must prove holds vacuously when
    /// it can never be reached, so only an error about the code itself is an error here
    /// (an unknown name, a malformed form), not a proof that could not be made.
    pub(super) fn check_unreachable(
        &mut self,
        context: SpanId,
        detail: &str,
        check: impl FnOnce(&mut Self) -> Result<Value>,
    ) -> Result<()> {
        self.check_arm_unreached(context, Some(detail), check)
    }

    /// Check code that cannot run, and say so when `detail` is given. An arm written for
    /// another target is expected to be unreachable on this one, so it is checked silently.
    pub(super) fn check_arm_unreached(
        &mut self,
        context: SpanId,
        detail: Option<&str>,
        check: impl FnOnce(&mut Self) -> Result<Value>,
    ) -> Result<()> {
        let names = self.proof.names.clone();
        let assignable = self.proof.assignable.clone();
        let owners = self.proof.owners.clone();
        let facts = self.proof.facts.clone();
        let scalars = self.proof.scalars.clone();
        let guards = self.proof.integer_guards.clone();
        let checked = self.conversions_checked;
        let events = self.conversion_events.len();
        let sites = self.world_sites.clone();
        let accepting = self.acceptances.len();
        // A `return` it runs ends no real path (a trial pass of a loop body that can return).
        let returned = (
            self.proof.returned,
            self.proof.returned_value,
            self.proof.result_sources.len(),
        );
        // Its choices are its own, so the replay of the real code is not disturbed.
        let choices = std::mem::take(&mut self.replay.choices);
        let flags = std::mem::take(&mut self.replay.outcome_flags);
        let cursor = std::mem::replace(&mut self.replay.choice_cursor, 0);
        let result = check(self);
        self.replay.choices = choices;
        self.replay.outcome_flags = flags;
        self.replay.choice_cursor = cursor;
        self.proof.names = names;
        self.proof.assignable = assignable;
        self.proof.owners = owners;
        self.proof.facts = facts;
        self.proof.scalars = scalars;
        self.proof.integer_guards = guards;
        self.conversions_checked = checked;
        self.conversion_events.truncate(events);
        self.world_sites = sites;
        self.acceptances.truncate(accepting);
        (self.proof.returned, self.proof.returned_value) = (returned.0, returned.1);
        self.proof.result_sources.truncate(returned.2);
        match result {
            Err(
                SourceCheckError::Type
                | SourceCheckError::Unproved
                | SourceCheckError::Overflow
                | SourceCheckError::UseAfterMove
                | SourceCheckError::Unconsumed
                | SourceCheckError::BorrowConflict
                | SourceCheckError::MoveOutOfBorrow
                | SourceCheckError::OverwriteLinear
                | SourceCheckError::EndEscape,
            )
            | Ok(_) => {}
            Err(error) => return Err(error),
        }
        if let Some(detail) = detail {
            self.note(
                SourceCheckNoteKind::UnreachableCode,
                context,
                detail.to_owned(),
            );
        }
        Ok(())
    }

    /// Forget what a loop body could have changed: every number that can be reassigned, and
    /// every record that could be lent `mut` (all but those only shared with us).
    pub(super) fn forget_loop_state(&mut self) -> Result<()> {
        self.forget_named(None)
    }

    /// The same, limited to `modified` when it is given: a local the loop cannot change keeps
    /// every fact, including the ones the enclosing conditions gave it.
    fn forget_named(&mut self, modified: Option<&[&'a str]>) -> Result<()> {
        self.charge(self.proof.names.len())?;
        for index in 0..self.proof.names.len() {
            let (name, value) = self.proof.names[index];
            if modified.is_some_and(|modified| !modified.contains(&name)) {
                continue;
            }
            let changeable = match value.ty {
                SourceType::Integer { .. } => self.proof.assignable.contains(&name),
                SourceType::Aggregate { .. } => {
                    value.available
                        && value.owner.is_some_and(|owner| {
                            self.proof.owners[owner].passing != calls::Passing::Ref
                        })
                }
                _ => false,
            };
            if changeable {
                self.havoc_local(index)?;
            }
        }
        Ok(())
    }

    fn loop_syntax(
        &mut self,
        expression: &'a Expression,
        loop_expression: &'a LoopExpression,
    ) -> Result<()> {
        let LoopExpression::For {
            for_keyword,
            binder,
            in_keyword,
            iterable,
            invariant,
            do_keyword,
            body,
        } = loop_expression
        else {
            return Err(SourceCheckError::Unsupported);
        };
        self.name(binder)?;
        for (span, keyword) in [
            (*for_keyword, "for"),
            (*in_keyword, "in"),
            (*do_keyword, "do"),
        ] {
            self.keyword(span, keyword)?;
        }
        let range = self.range_of(expression.span_id);
        let mut cursor = range.start;
        let mut parts = vec![*for_keyword, binder.span_id, *in_keyword, iterable.span_id];
        if let Some(invariant) = invariant {
            self.keyword(invariant.where_keyword, "where")?;
            parts.push(invariant.where_keyword);
            parts.push(invariant.condition.span_id);
        }
        parts.extend([*do_keyword, body.span_id]);
        for span in parts {
            self.child(expression.span_id, span)?;
            let part = self.range_of(span);
            self.syntax_trivia(part.file, cursor, part.start)?;
            cursor = part.end;
        }
        self.syntax_trivia(range.file, cursor, range.end)
    }

    /// Unit has no element type. Accept only a written no-op, never a binder witness.
    fn loop_empty_body(&mut self, root: &'a Expression) -> Result<()> {
        let mut pending = vec![root];
        while let Some(body) = pending.pop() {
            self.charge(1)?;
            match &body.value {
                Expr::Group(child) => {
                    self.group_syntax(body, child)?;
                    pending.push(child);
                }
                Expr::Sequence(sequence) if sequence.elements.is_empty() => {
                    self.sequence_syntax(body, sequence)?;
                }
                Expr::Block(block) => {
                    self.block_syntax(body, block)?;
                    self.charge(block.statements.len())?;
                    for statement in &block.statements {
                        let Statement::Expression(child) = &statement.value else {
                            return Err(SourceCheckError::Unsupported);
                        };
                        pending.push(child);
                    }
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(())
    }

    /// Inspect direct source only. Returns inside an ordinary checked callee are not loop exits.
    /// Also reports whether the body can branch, which decides how far a loop may unroll.
    fn loop_exit_free(&mut self, root: &'a Expression, exits: bool) -> Result<bool> {
        Ok(self.loop_scan(root, exits)?.branches)
    }

    /// Whether a loop body branches or ends the path, and which locals it could change: every
    /// place it assigns, and every place it hands to a call (a `mut` argument or receiver is a
    /// place, so its root name appears). Other locals keep what is known across a pass.
    fn loop_scan(&mut self, root: &'a Expression, exits: bool) -> Result<LoopScan<'a>> {
        let mut scan = LoopScan::default();
        let mut pending = vec![root];
        while let Some(expression) = pending.pop() {
            self.charge(1)?;
            scan.branches |= matches!(
                expression.value,
                Expr::Conditional(_) | Expr::Match(_) | Expr::Loop(_)
            );
            scan.ends_path |= matches!(expression.value, Expr::Loop(_));
            self.loop_scan_expression(expression, &mut pending, &mut scan, exits)?;
        }
        Ok(scan)
    }

    fn loop_scan_expression(
        &mut self,
        expression: &'a Expression,
        pending: &mut Vec<&'a Expression>,
        scan: &mut LoopScan<'a>,
        exits: bool,
    ) -> Result<()> {
        match &expression.value {
            Expr::Name(_) | Expr::Literal(_) => {}
            Expr::Group(child)
            | Expr::Unary { operand: child, .. }
            | Expr::Cast { value: child, .. } => {
                self.charge(1)?;
                pending.push(child);
            }
            // `#unchecked(x)` is `x` to a loop: the directive is not a call that could change a
            // place, so its operand is scanned like a group's.
            Expr::Call { .. } if let Some(child) = unchecked_operand(expression) => {
                self.charge(1)?;
                pending.push(child);
            }
            Expr::Member { base: child, name } => {
                self.charge(1)?;
                pending.push(child);
                // `b.add` written without parentheses can be a call that lends `b` `mut`: the
                // receiver's type is not known here, so it counts when any method of that name
                // takes something `mut`.
                self.charge(self.module.function_names.len())?;
                let lends = self.module.function_names.iter().any(|(symbol, index)| {
                    symbol
                        .rsplit_once('.')
                        .is_some_and(|(_, method)| method == name.value)
                        && self
                            .module
                            .functions
                            .get(*index)
                            .and_then(|function| function.as_ref())
                            .is_some_and(|function| !function.signature.mutated.is_empty())
                });
                if lends {
                    scan.modified
                        .extend(assign::split_place(child).map(|(root, _)| root));
                }
            }
            Expr::Binary { left, right, .. }
            | Expr::Range {
                start: left,
                end: right,
                ..
            } => {
                self.charge(2)?;
                pending.extend([left.as_ref(), right.as_ref()]);
            }
            Expr::Sequence(sequence) => {
                self.charge(sequence.elements.len())?;
                pending.extend(&sequence.elements);
            }
            Expr::Array(array) => {
                self.charge(array.elements.len() + 1)?;
                pending.extend(&array.elements);
                if let Some(length) = &array.length {
                    pending.push(&length.value);
                }
            }
            Expr::Select(select) => {
                self.charge(2)?;
                pending.extend([&select.base, &select.index]);
                if let Some(length) = &select.length {
                    pending.push(&length.value);
                }
            }
            Expr::NumericSelect(select) => {
                self.charge(1)?;
                pending.push(&select.base);
            }
            Expr::Call { callee, arguments } => {
                self.charge(arguments.value.len() + 1)?;
                pending.push(callee);
                scan.modified
                    .extend(assign::split_place(callee).map(|(root, _)| root));
                for argument in &arguments.value {
                    let (Argument::Positional(value)
                    | Argument::Named { value, .. }
                    | Argument::Typed {
                        annotation: value, ..
                    }) = &argument.value;
                    pending.push(value);
                    scan.modified
                        .extend(assign::split_place(value).map(|(root, _)| root));
                }
            }
            Expr::Conditional(case) => {
                self.charge(3)?;
                pending.extend([&case.condition, &case.then_branch, &case.else_branch]);
            }
            Expr::Match(case) => {
                self.charge(case.arms.value.len() + 1)?;
                pending.push(&case.subject);
                pending.extend(case.arms.value.iter().map(|arm| &arm.body));
            }
            Expr::Loop(loop_expression) => match loop_expression.as_ref() {
                LoopExpression::For { iterable, body, .. } => {
                    self.charge(2)?;
                    pending.extend([iterable, body]);
                }
                LoopExpression::While {
                    condition, body, ..
                } => {
                    self.charge(2)?;
                    pending.extend([condition, body]);
                }
            },
            Expr::Block(block) => self.loop_scan_block(block, pending, scan, exits)?,
            // A test, a chain, a bound value and a query change nothing but what they read.
            Expr::IsTest(test) => {
                self.charge(1)?;
                pending.push(&test.subject);
            }
            Expr::Chain(chain) => {
                self.charge(chain.arms.len() + 1)?;
                for arm in &chain.arms {
                    pending.extend([&arm.value.condition, &arm.value.value]);
                }
                pending.push(&chain.otherwise);
            }
            Expr::Bind(binding) => {
                if let Some(value) = &binding.value {
                    self.charge(1)?;
                    pending.push(&value.expression);
                }
            }
            Expr::Query { .. } | Expr::Hole => {}
            _ => return Err(SourceCheckError::Unsupported),
        }
        Ok(())
    }

    fn loop_scan_block(
        &mut self,
        block: &'a Block,
        pending: &mut Vec<&'a Expression>,
        scan: &mut LoopScan<'a>,
        exits: bool,
    ) -> Result<()> {
        self.charge(block.statements.len())?;
        for statement in &block.statements {
            match &statement.value {
                Statement::Error => return Err(SourceCheckError::Unsupported),
                // A `return` ends the path from inside the loop. Only `while` and range loops
                // may hold one; a loop over a sequence consumes it and cannot.
                Statement::Return(returned) => {
                    if !exits {
                        return Err(SourceCheckError::Unsupported);
                    }
                    scan.ends_path = true;
                    pending.extend(&returned.value);
                }
                // A fall-through conditional joins its branches, so it does not branch the
                // loop's worlds; one that returns is an exit, which loops do not allow yet.
                Statement::WhenIs(when) => {
                    let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                    pending.push(&when.subject);
                    for block in std::iter::once(&when.then_block).chain(otherwise) {
                        if guards::returns(&block.value) && !exits {
                            return Err(SourceCheckError::Unsupported);
                        }
                        self.loop_scan_block(&block.value, pending, scan, exits)?;
                    }
                }
                Statement::When(when) => {
                    let otherwise = when.else_branch.iter().map(|branch| &branch.block);
                    pending.push(&when.condition);
                    for block in std::iter::once(&when.then_block).chain(otherwise) {
                        if guards::returns(&block.value) && !exits {
                            return Err(SourceCheckError::Unsupported);
                        }
                        self.loop_scan_block(&block.value, pending, scan, exits)?;
                    }
                }
                Statement::Assign(assign) => {
                    scan.modified
                        .extend(assign::split_place(&assign.place).map(|(root, _)| root));
                    pending.extend([&assign.place, &assign.value]);
                }
                Statement::Expression(value) => pending.push(value),
                Statement::Binding(binding) => {
                    if let Some(body) = &binding.value {
                        pending.push(&body.expression);
                    }
                }
                Statement::Destructure(pattern) => pending.push(&pattern.value),
            }
        }
        Ok(())
    }

    pub(super) fn loop_next(
        &mut self,
        frame: Box<Frame<'a>>,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        self.charge_flow()?;
        self.charge(1)?;
        let owner = frame.iterable.owner.ok_or(SourceCheckError::Type)?;
        assert!(frame.next <= frame.count);
        assert!(!self.proof.owners[owner].drained);
        if frame.next == frame.count {
            self.proof.owners[owner].drained = true;
            self.release(frame.iterable)?;
            values.push(Value::unit());
            return Ok(());
        }
        let child = self.array_child(frame.iterable, frame.next)?;
        self.charge(self.proof.owners[owner].moved.len() + 1)?;
        assert_eq!(
            self.proof.owners[owner].moved.last().copied(),
            frame.next.checked_sub(1)
        );
        self.proof.owners[owner].moved.push(frame.next);
        let iteration = self.loop_iteration(frame, child, values.len())?;
        let body = iteration.frame.body;
        pending.push(EvalTask::LoopEnd(Box::new(iteration)));
        pending.push(EvalTask::Expression(body, Some(SourceType::Unit), false));
        Ok(())
    }

    fn loop_iteration(
        &mut self,
        frame: Box<Frame<'a>>,
        child: Value,
        values_before: usize,
    ) -> Result<Iteration<'a>> {
        let owners_before = self.proof.owners.len();
        let current = self.loop_current(child, owners_before)?;
        let inventory = self.loop_inventory(&frame, &current)?;
        self.charge(self.proof.names.len() * 2)?;
        let names = self.proof.names.clone();
        self.proof.names.retain(|(name, _)| *name != frame.binder);
        self.insert(frame.binder, child)?;
        Ok(Iteration {
            frame,
            names,
            inventory,
            current,
            owners_before,
            values_before,
        })
    }

    fn loop_current(&mut self, child: Value, count: usize) -> Result<Vec<bool>> {
        self.charge(count + 1)?;
        let mut current = vec![false; count];
        let mut pending: Vec<_> = child.owner.into_iter().collect();
        while let Some(owner) = pending.pop() {
            self.charge(1)?;
            assert!(owner < count);
            if current[owner] {
                continue;
            }
            current[owner] = true;
            self.charge(self.proof.owners[owner].children.len())?;
            pending.extend(
                self.proof.owners[owner]
                    .children
                    .iter()
                    .filter_map(|child| child.owner),
            );
        }
        Ok(current)
    }

    fn loop_inventory(&mut self, frame: &Frame<'a>, current: &[bool]) -> Result<Vec<FramedOwner>> {
        assert_eq!(current.len(), self.proof.owners.len());
        let root = frame.iterable.owner.ok_or(SourceCheckError::Type)?;
        let mut inventory = Vec::new();
        for (id, selected) in current.iter().enumerate() {
            self.charge(1)?;
            if id == root || *selected {
                continue;
            }
            self.charge(self.proof.owners[id].moved.len() + 1)?;
            inventory.push(FramedOwner {
                id,
                live: self.proof.owners[id].live,
                moved: self.proof.owners[id].moved.clone(),
                drained: self.proof.owners[id].drained,
            });
        }
        Ok(inventory)
    }

    pub(super) fn loop_end(
        &mut self,
        mut iteration: Box<Iteration<'a>>,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        self.charge_flow()?;
        assert_eq!(values.len(), iteration.values_before + 1);
        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
        if value.ty != SourceType::Unit {
            return Err(SourceCheckError::Type);
        }
        self.loop_accounted(&iteration)?;
        // A final normal exit may consume captures once. Only an actual backedge
        // requires their old availability; scope restoration below never revives them.
        if iteration.frame.next + 1 < iteration.frame.count {
            self.loop_captures(&iteration)?;
        }
        let binder = iteration.frame.binder;
        self.end_arm_scope(iteration.names, &[binder])?;
        iteration.frame.next += 1;
        pending.push(EvalTask::LoopNext(iteration.frame));
        Ok(())
    }

    fn loop_accounted(&mut self, iteration: &Iteration<'a>) -> Result<()> {
        self.charge(self.proof.owners.len())?;
        for (owner, current) in iteration.current.iter().enumerate() {
            if *current && self.proof.owners[owner].live {
                return Err(SourceCheckError::Unconsumed);
            }
        }
        if self.proof.owners[iteration.owners_before..]
            .iter()
            .any(|owner| owner.live)
        {
            return Err(SourceCheckError::Unconsumed);
        }
        self.charge_product(self.proof.names.len(), iteration.names.len() + 1)?;
        for (name, value) in &self.proof.names {
            let local = *name == iteration.frame.binder
                || !iteration.names.iter().any(|(outer, _)| outer == name);
            if local
                && value.available
                && value
                    .owner
                    .is_some_and(|owner| self.proof.owners[owner].live)
            {
                return Err(SourceCheckError::Unconsumed);
            }
        }
        Ok(())
    }

    fn loop_captures(&mut self, iteration: &Iteration<'a>) -> Result<()> {
        self.charge_product(iteration.names.len(), self.proof.names.len() + 1)?;
        for (name, before) in &iteration.names {
            if *name == iteration.frame.binder {
                continue;
            }
            let after = self
                .proof
                .names
                .iter()
                .find(|(current, _)| current == name)
                .map(|(_, value)| value)
                .ok_or(SourceCheckError::Malformed)?;
            if before.available != after.available
                || before.owner != after.owner
                || before.identity != after.identity
            {
                return Err(SourceCheckError::UseAfterMove);
            }
        }
        for before in &iteration.inventory {
            self.charge(before.moved.len() + 1)?;
            let after = &self.proof.owners[before.id];
            if before.live != after.live
                || before.drained != after.drained
                || before.moved != after.moved
            {
                return Err(SourceCheckError::UseAfterMove);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) -> Result<CheckedSourceModule> {
        let source = parser::parse_strict(text).unwrap();
        check_module("loops", &source, &[], crate::Limits::default())
    }

    const TOKEN: &str = concat!(
        "Token has number in 0 to 16\n",
        "consume(value Token):\n",
        "  Token(number) := value\n",
        "  return\n",
    );

    #[test]
    fn fixed_consuming_loop_drains_original_positions_and_exports_only_flat_origins() {
        let text = format!(
            "{TOKEN}{}",
            concat!(
                "drain(values (Token, Token)):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "  return\n"
            )
        );
        let module = check(&text).unwrap();
        let summaries = module.summaries("drain");
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].outputs.is_empty());
        assert_eq!(summaries[0].inputs[0].slots, [0, 1]);
        assert_eq!(
            summaries[0].dispositions[0],
            summaries::Disposition::Unpacked
        );
        let source = parser::parse_strict(concat!(
            "use loops Token, drain\n",
            "client():\n",
            "  values: (Token(number: 1), Token(number: 2))\n",
            "  drain(values)\n",
            "  return\n",
        ))
        .unwrap();
        assert!(check_module("client", &source, &[&module], crate::Limits::default()).is_ok());
    }

    #[test]
    fn captured_ownership_is_framed_only_at_actual_backedges() {
        let prefix = format!(
            "{TOKEN}{}",
            concat!(
                "f(values (Token,), outer Token):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "    consume(outer)\n"
            )
        );
        let singleton = format!("{prefix}  return\n");
        assert!(check(&singleton).is_ok());
        let repeated = singleton.replace("(Token,)", "(Token, Token)");
        assert_eq!(
            check(&repeated).unwrap_err(),
            SourceCheckError::UseAfterMove
        );
        let text = format!("{prefix}  consume(outer)\n  return\n");
        assert_eq!(check(&text).unwrap_err(), SourceCheckError::UseAfterMove);
    }

    #[test]
    fn body_and_current_element_owners_cannot_leak_past_any_step() {
        for body in [
            "    ()\n",
            "    local: value\n",
            "    consume(value)\n    local: Token(number: 1)\n",
        ] {
            let text =
                format!("{TOKEN}f(values (Token,)):\n  for value in values do\n{body}  return\n");
            assert_eq!(check(&text).unwrap_err(), SourceCheckError::Unconsumed);
        }
        let text = format!(
            "{TOKEN}{}",
            concat!(
                "f(values (Token,), outer Token):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "    alias: outer\n",
                "  return\n"
            )
        );
        assert_eq!(check(&text).unwrap_err(), SourceCheckError::Unconsumed);
    }

    #[test]
    fn heterogeneous_nested_sequences_check_each_original_position_type() {
        let text = concat!(
            "Small is A or B\nBig is A or B or C\n",
            "Small has\n",
            "  consume(self):\n",
            "    when self is\n",
            "      A then ()\n",
            "      B then ()\n",
            "    return\n",
            "Big has\n",
            "  consume(self):\n",
            "    when self is\n",
            "      A then ()\n",
            "      B then ()\n",
            "      C then ()\n",
            "    return\n",
            "drain(values ((Small,), (Big, Big))):\n",
            "  for row in values do\n",
            "    for value in row do\n",
            "      value.consume\n",
            "  return\n",
        );
        let module = check(text).unwrap();
        assert_eq!(module.summaries("drain").len(), 18);
        let bad = format!(
            "{TOKEN}{}",
            concat!(
                "f(values (Token, Token)):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "    consume(value)\n",
                "  return\n"
            )
        );
        assert_eq!(check(&bad).unwrap_err(), SourceCheckError::UseAfterMove);
    }

    #[test]
    fn empty_iteration_has_no_element_witness_or_unchecked_body_effects() {
        assert!(check("f():\n  for value in () do\n    (())\n  return\n").is_ok());
        assert!(check("f():\n  for value in () do\n    ()\n    (())\n  return\n").is_ok());
        for body in ["unknown", "value", "call()", "(0,)", "return"] {
            let text = format!("f():\n  for value in () do\n    {body}\n  return\n");
            assert_eq!(
                check(&text).unwrap_err(),
                SourceCheckError::Unsupported,
                "{body}"
            );
        }
        assert!(check("f():\n  for value in (1, 2) do\n    ()\n  return\n").is_ok());
    }

    #[test]
    fn returns_inside_loop_source_are_unsupported_but_checked_helper_returns_are_allowed() {
        for body in [
            "return",
            "return value",
            "when value is\n      Token(number) then\n        return",
        ] {
            let text = format!(
                "{TOKEN}f(values (Token,)):\n  for value in values do\n    {body}\n  return\n"
            );
            assert_eq!(
                check(&text).unwrap_err(),
                SourceCheckError::Unsupported,
                "{body}"
            );
        }
    }

    #[test]
    fn repeated_scalar_origins_keep_guards_correlated_without_framing_scalar_facts() {
        let text = concat!(
            "drain(value in 0 to 1):\n",
            "  for element in (value, value) do\n",
            "    when element is\n",
            "      0 then ()\n",
            "      1 then ()\n",
            "  return\n",
        );
        let source = parser::parse_strict(text).unwrap();
        let library = check_module("library", &source, &[], crate::Limits::default()).unwrap();
        assert_eq!(library.summaries("drain").len(), 2);
        drop(source);
        let source = parser::parse_strict(concat!(
            "use library drain\n",
            "observe(value in 0 to 1) in 0 to 1 and = value:\n",
            "  drain(value)\n",
            "  return value\n",
        ))
        .unwrap();
        let client =
            check_module("client", &source, &[&library], crate::Limits::default()).unwrap();
        assert_eq!(client.summaries("observe").len(), 2);
    }

    #[test]
    fn an_empty_body_node_cannot_hide_nontrivia_written_body_source() {
        let mut source = parser::parse_strict(concat!(
            "f():\n",
            "  for value in () do\n",
            "    unknown\n",
            "  return\n",
        ))
        .unwrap();
        let Item::Binding(binding) = &mut source.items[0].value else {
            unreachable!()
        };
        let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
            unreachable!()
        };
        let Statement::Expression(expression) = &mut block.statements[0].value else {
            unreachable!()
        };
        let Expr::Loop(loop_expression) = &mut expression.value else {
            unreachable!()
        };
        let LoopExpression::For { body, .. } = loop_expression.as_mut() else {
            unreachable!()
        };
        let Expr::Block(block) = &mut body.value else {
            unreachable!()
        };
        block.statements.clear();
        assert_eq!(
            check_file(&source, crate::Limits::default()),
            Err(SourceCheckError::Malformed)
        );
    }

    #[test]
    fn loop_blocks_cannot_erase_or_reclassify_written_returns() {
        let text = format!(
            "{TOKEN}{}",
            concat!(
                "f(values (Token, Token)):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "    return ()\n",
                "  return\n",
            )
        );
        for erase in [false, true] {
            let mut source = parser::parse_strict(&text).unwrap();
            let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
                unreachable!()
            };
            let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
                unreachable!()
            };
            let Statement::Expression(expression) = &mut block.statements[0].value else {
                unreachable!()
            };
            let Expr::Loop(loop_expression) = &mut expression.value else {
                unreachable!()
            };
            let LoopExpression::For { body, .. } = loop_expression.as_mut() else {
                unreachable!()
            };
            let Expr::Block(block) = &mut body.value else {
                unreachable!()
            };
            if erase {
                block.statements.pop();
            } else {
                let statement = block.statements.last_mut().unwrap();
                let Statement::Return(returned) = &mut statement.value else {
                    unreachable!()
                };
                statement.value = Statement::Expression(returned.value.take().unwrap());
            }
            assert_eq!(
                check_file(&source, crate::Limits::default()),
                Err(SourceCheckError::Malformed),
                "erase={erase}"
            );
        }
    }

    #[test]
    fn consuming_loop_work_and_flow_limits_have_exact_fail_closed_boundaries() {
        let text = format!(
            "{TOKEN}{}",
            concat!(
                "drain(values (Token, Token)):\n",
                "  for value in values do\n",
                "    consume(value)\n",
                "  return\n"
            )
        );
        let source = parser::parse_strict(&text).unwrap();
        for flow in [false, true] {
            let mut lower = 0;
            let mut upper = if flow {
                crate::Limits::default().flow_work_max
            } else {
                crate::Limits::default().work_max
            };
            for _ in 0..20 {
                if lower == upper {
                    break;
                }
                let middle = lower + (upper - lower) / 2;
                let mut limits = crate::Limits::default();
                if flow {
                    limits.flow_work_max = middle;
                } else {
                    limits.work_max = middle;
                }
                match check_file(&source, limits) {
                    Ok(_) => upper = middle,
                    Err(SourceCheckError::Budget) => lower = middle + 1,
                    other => panic!("{other:?}"),
                }
            }
            assert_eq!(lower, upper);
            assert!(lower > 0);
            let mut limits = crate::Limits::default();
            if flow {
                limits.flow_work_max = lower - 1;
            } else {
                limits.work_max = lower - 1;
            }
            assert_eq!(check_file(&source, limits), Err(SourceCheckError::Budget));
        }
    }
}

/// A number inside a local record: which local, the fields to it, and the number at the entry.
type FrameLeaf = (usize, Vec<usize>, Value);

impl<'a> Checker<'a> {
    /// The numbers inside the records of `modified` locals, with their values now.
    fn frame_leaves(&mut self, modified: &[&'a str]) -> Vec<FrameLeaf> {
        let mut found = Vec::new();
        for index in 0..self.proof.names.len() {
            let (name, value) = self.proof.names[index];
            if !modified.contains(&name) || !matches!(value.ty, SourceType::Aggregate { .. }) {
                continue;
            }
            let mut leaves = Vec::new();
            if self
                .collect_leaves(value, &mut Vec::new(), &mut leaves, 0)
                .is_ok()
            {
                found.extend(leaves.into_iter().map(|(path, leaf)| (index, path, leaf)));
            }
        }
        found
    }

    fn collect_leaves(
        &mut self,
        value: Value,
        path: &mut Vec<usize>,
        out: &mut Vec<(Vec<usize>, Value)>,
        depth: u32,
    ) -> Result<()> {
        if depth > 6 || out.len() > 64 {
            return Ok(());
        }
        match value.ty {
            SourceType::Integer { .. } => {
                if value.identity.is_some() {
                    out.push((path.clone(), value));
                }
            }
            SourceType::Aggregate { .. } if self.domain_bounds_of(value.ty).is_none() => {
                let Some(owner) = value.owner else {
                    return Ok(());
                };
                if !value.available {
                    return Ok(());
                }
                self.materialize_children(value)?;
                let children = self.proof.owners[owner].children.clone();
                for (index, child) in children.into_iter().enumerate() {
                    path.push(index);
                    self.collect_leaves(child, path, out, depth + 1)?;
                    path.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The number at `path` inside `value`, when it is still there to be read.
    fn leaf_at(&mut self, mut value: Value, path: &[usize]) -> Result<Option<Value>> {
        for &index in path {
            let Some(owner) = value.owner else {
                return Ok(None);
            };
            if !matches!(value.ty, SourceType::Aggregate { .. }) {
                return Ok(None);
            }
            self.materialize_children(value)?;
            let Some(child) = self.proof.owners[owner].children.get(index).copied() else {
                return Ok(None);
            };
            value = child;
        }
        Ok(value.identity.map(|_| value))
    }

    /// Which of `entry` a pass of the loop body gives back as it was given: the body is checked
    /// once (and its outcome thrown away) from a head that knows all of them, and the ones whose
    /// value is the same value at its end are kept. The real pass assumes them and proves it.
    fn unchanged_leaves(
        &mut self,
        frame: &RangeFrame<'a>,
        start: Value,
        end: Value,
        modified: &[&'a str],
        invariant: &[Constraint],
        entry: &[FrameLeaf],
    ) -> Result<Vec<FrameLeaf>> {
        self.unchanged_leaves_with(frame.body.span_id, modified, entry, |this| {
            let counter = this.fresh_value(SourceType::Integer {
                lower: start.lower,
                upper: end.upper,
            })?;
            this.assume(BinaryOperator::GreaterOrEqual, counter, start)?;
            this.assume(BinaryOperator::LessOrEqual, counter, end)?;
            this.insert(frame.binder, counter)?;
            this.assume_invariant(invariant)?;
            this.loop_body(frame.body)
        })
    }

    /// [`Self::unchanged_leaves`] for any loop: `pass` sets up the head and runs one pass.
    fn unchanged_leaves_with(
        &mut self,
        span: SpanId,
        modified: &[&'a str],
        entry: &[FrameLeaf],
        pass: impl FnOnce(&mut Self) -> Result<Value>,
    ) -> Result<Vec<FrameLeaf>> {
        let mut kept: Vec<FrameLeaf> = Vec::new();
        let trial = self.check_arm_unreached(span, None, |this| {
            this.forget_named(Some(modified))?;
            let mut head = Vec::new();
            for (index, path, was) in entry {
                let local = this.proof.names[*index].1;
                if let Some(leaf) = this.leaf_at(local, path)? {
                    let (was, leaf) = (this.ensure_identity(*was)?, this.ensure_identity(leaf)?);
                    this.assume(BinaryOperator::Equal, leaf, was)?;
                    head.push((*index, path.clone(), leaf, was));
                }
            }
            let result = pass(this)?;
            // A pass that returned says nothing of what a pass that goes on leaves.
            if this.proof.returned {
                return Ok(result);
            }
            for (index, path, leaf, was) in head {
                let local = this.proof.names[index].1;
                if let Some(after) = this.leaf_at(local, &path)?
                    && after.identity == leaf.identity
                {
                    kept.push((index, path, was));
                }
            }
            Ok(result)
        });
        trial?;
        Ok(kept)
    }

    /// At a head (and at the loop's end): each kept number is the number it was at the entry.
    /// Returns the numbers as they stand, for the end of a pass to be held against.
    fn assume_frame(&mut self, kept: &[FrameLeaf]) -> Result<Vec<FrameLeaf>> {
        let mut heads = Vec::new();
        for (index, path, was) in kept {
            let local = self.proof.names[*index].1;
            if let Some(leaf) = self.leaf_at(local, path)? {
                let (was, leaf) = (self.ensure_identity(*was)?, self.ensure_identity(leaf)?);
                self.assume(BinaryOperator::Equal, leaf, was)?;
                heads.push((*index, path.clone(), leaf));
            }
        }
        Ok(heads)
    }

    /// At the end of a pass each kept number is still the one the pass began with.
    fn prove_frame(&mut self, heads: &[FrameLeaf]) -> Result<()> {
        for (index, path, head) in heads {
            let local = self.proof.names[*index].1;
            let Some(after) = self.leaf_at(local, path)? else {
                return Err(SourceCheckError::Unproved);
            };
            self.compare(BinaryOperator::Equal, after, *head)?;
        }
        Ok(())
    }
}
