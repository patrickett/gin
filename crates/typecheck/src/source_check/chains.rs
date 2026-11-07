//! Compound conditions and the forms built on them: a condition chain (`when` with ordered
//! `condition then value` arms ending in `else value`), a guard on a case arm (`Pattern and
//! condition then value`), and one-line conditionals whose condition combines `is` tests with
//! `and`, `or` and `not`.
//!
//! A condition is flattened to terms under one connective (negation pushed inward, as in
//! `guards::conditions`), and the terms are tried in source order. Each term is a replay choice:
//! a comparison picks one of the sides its intervals allow and assumes it, and a variant test
//! picks the subject's member. So a condition of `n` terms costs `n + 1` worlds at most, not
//! `2^n`. When the condition holds the site's value runs; when it fails the site falls to its
//! other side: the `else` of a conditional, the next arm of a chain, the next arm of a case.
//!
//! The binders of an `is` test are in scope only where the test held: in a conjunction they are
//! visible to the later terms and to the value; in a disjunction or under `not` nothing could
//! bind, so those tests may not name binders.
use super::*;
use aggregates::EvalTask;
use guards::{Connective, Leaf, Relation};
use sums::ValidationState;

#[derive(Clone, Copy)]
pub(super) enum Term<'a> {
    Relation(Leaf<'a>),
    Variant { negated: bool, test: &'a IsTest },
}

/// What a condition decides.
pub(super) enum Site<'a> {
    /// `when c then a else b`: `a` where it holds and `b` where it does not.
    Conditional {
        then: &'a Expression,
        otherwise: &'a Expression,
    },
    /// Arm `index` of the chain: its value where the condition holds, the next arm where not.
    Chain {
        expression: &'a Expression,
        index: usize,
    },
    /// The guard of a case arm. The arm's body and scope are already pending, `base` deep. Where
    /// the guard fails they are dropped and the state before the arm returns for the next arm
    /// of the same member. `snapshot` is `None` when the arm is only checked, not selected.
    Guard {
        base: usize,
        rest: Vec<&'a MatchArm>,
        subject: Value,
        snapshot: Option<Box<ValidationState<'a>>>,
    },
}

pub(super) struct CondFrame<'a> {
    terms: Vec<Term<'a>>,
    connective: Connective,
    index: usize,
    operands: bool,
    site: Site<'a>,
    expected: Option<SourceType>,
    scope: Option<(Vec<(&'a str, Value)>, Vec<&'a str>)>,
}

/// The terms of a condition in source order and how they combine; `None` for a condition made
/// of anything else, or one that mixes `and` with `or`.
pub(super) fn terms(condition: &Expression) -> Option<(Connective, Vec<Term<'_>>)> {
    let mut found = Vec::new();
    let mut connective = None;
    let mut stack = vec![(condition, false)];
    while let Some((next, negated)) = stack.pop() {
        match &next.value {
            Expr::Group(inner) => stack.push((inner, negated)),
            Expr::Unary { operator, operand } if operator.value == UnaryOperator::Not => {
                stack.push((operand, !negated));
            }
            Expr::Binary {
                left,
                operator,
                right,
            } if matches!(operator.value, BinaryOperator::And | BinaryOperator::Or) => {
                let this = match (operator.value, negated) {
                    (BinaryOperator::And, false) | (BinaryOperator::Or, true) => Connective::And,
                    _ => Connective::Or,
                };
                if *connective.get_or_insert(this) != this {
                    return None;
                }
                stack.push((right, negated));
                stack.push((left, negated));
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let relation = Relation::from_binary(operator.value)?;
                found.push(Term::Relation(Leaf {
                    left,
                    right: guards::Side::Expr(right),
                    relation: if negated { relation.negate() } else { relation },
                }));
            }
            Expr::IsTest(test) => match &test.pattern.value {
                MatchPattern::Integer(number) => found.push(Term::Relation(Leaf {
                    left: &test.subject,
                    right: guards::Side::Expr(number),
                    relation: if negated {
                        Relation::NotEqual
                    } else {
                        Relation::Equal
                    },
                })),
                MatchPattern::Domain { domain, .. } => found.push(Term::Relation(Leaf {
                    left: &test.subject,
                    right: guards::Side::Domain(domain),
                    relation: if negated {
                        Relation::Outside
                    } else {
                        Relation::Within
                    },
                })),
                MatchPattern::Variant(_) => found.push(Term::Variant { negated, test }),
                MatchPattern::Text(_) | MatchPattern::Else { .. } => return None,
            },
            _ => return None,
        }
    }
    Some((connective.unwrap_or(Connective::And), found))
}

/// Whether a condition holds a test of a variant, however wrapped.
pub(super) fn mentions_variant(condition: &Expression) -> bool {
    let mut stack = vec![condition];
    while let Some(next) = stack.pop() {
        match &next.value {
            Expr::Group(inner) => stack.push(inner),
            Expr::Unary { operand, .. } => stack.push(operand),
            Expr::Binary { left, right, .. } => stack.extend([left.as_ref(), right.as_ref()]),
            Expr::IsTest(test) if matches!(test.pattern.value, MatchPattern::Variant(_)) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

impl<'a> Checker<'a> {
    /// `when c then a else b` whose condition combines variant tests: see the module comment.
    pub(super) fn compound_conditional(
        &mut self,
        expression: &'a Expression,
        case: &'a ConditionalExpression,
        expected: Option<SourceType>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.child(expression.span_id, case.condition.span_id)?;
        self.child(expression.span_id, case.then_branch.span_id)?;
        self.child(expression.span_id, case.else_branch.span_id)?;
        self.token(case.when_keyword, "when")?;
        self.token(case.then_keyword, "then")?;
        self.token(case.else_keyword, "else")?;
        self.charge(3)?;
        let site = Site::Conditional {
            then: &case.then_branch,
            otherwise: &case.else_branch,
        };
        let frame = self.cond_start(&case.condition, site, expected)?;
        pending.push(EvalTask::Cond(Box::new(frame)));
        Ok(())
    }

    /// Arm `index` of a chain, with the earlier arms' conditions assumed false.
    pub(super) fn chain_tasks(
        &mut self,
        expression: &'a Expression,
        index: usize,
        expected: Option<SourceType>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        let Expr::Chain(chain) = &expression.value else {
            return Err(SourceCheckError::Malformed);
        };
        self.charge(4)?;
        if index == 0 {
            if chain.arms.is_empty() {
                return Err(SourceCheckError::Malformed);
            }
            self.child(expression.span_id, chain.when_keyword)?;
            self.keyword(chain.when_keyword, "when")?;
            self.child(expression.span_id, chain.else_keyword)?;
            self.keyword(chain.else_keyword, "else")?;
            self.child(expression.span_id, chain.otherwise.span_id)?;
        }
        let Some(arm) = chain.arms.get(index) else {
            pending.push(EvalTask::Expression(&chain.otherwise, expected, false));
            return Ok(());
        };
        self.child(expression.span_id, arm.span_id)?;
        self.child(arm.span_id, arm.condition.span_id)?;
        self.child(arm.span_id, arm.then_keyword)?;
        self.keyword(arm.then_keyword, "then")?;
        self.child(arm.span_id, arm.value.value.span_id)?;
        let site = Site::Chain { expression, index };
        let frame = self.cond_start(&arm.condition, site, expected)?;
        pending.push(EvalTask::Cond(Box::new(frame)));
        Ok(())
    }

    /// A case arm with a guard, selected (`rest` holds the arms to try where the guard fails) or
    /// only checked (`rest` is `None`: the guard is taken to hold).
    pub(super) fn guarded_arm(
        &mut self,
        arm: &'a MatchArm,
        subject: Value,
        expected: Option<SourceType>,
        rest: Option<Vec<&'a MatchArm>>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        let guard = arm.guard.as_ref().ok_or(SourceCheckError::Malformed)?;
        let snapshot = match &rest {
            Some(_) => Some(Box::new(self.validation_state()?)),
            None => None,
        };
        self.arm_tasks(arm, subject, expected, pending)?;
        let base = pending.len();
        // The arm's scope end and its body, in that order beneath the guard.
        if base < 2 || !matches!(pending[base - 2], EvalTask::Scope(..)) {
            return Err(SourceCheckError::Malformed);
        }
        let site = Site::Guard {
            base,
            rest: rest.unwrap_or_default(),
            subject,
            snapshot,
        };
        let frame = self.cond_start(&guard.condition, site, expected)?;
        pending.push(EvalTask::Cond(Box::new(frame)));
        Ok(())
    }

    /// The arms a subject's member can take, in order: the first whose guard holds, or the last.
    pub(super) fn candidate_tasks(
        &mut self,
        mut candidates: Vec<&'a MatchArm>,
        subject: Value,
        expected: Option<SourceType>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.charge(candidates.len())?;
        if candidates.is_empty() {
            return Err(SourceCheckError::Type);
        }
        let first = candidates.remove(0);
        if first.guard.is_some() {
            self.guarded_arm(first, subject, expected, Some(candidates), pending)
        } else {
            self.arm_tasks(first, subject, expected, pending)
        }
    }

    fn cond_start(
        &mut self,
        condition: &'a Expression,
        site: Site<'a>,
        expected: Option<SourceType>,
    ) -> Result<CondFrame<'a>> {
        self.condition_syntax(condition)?;
        let (connective, found) = terms(condition).ok_or_else(|| {
            self.explain(
                SourceCheckError::Unsupported,
                "a condition combines comparisons and `is` tests with either `and` or `or`, not both"
                    .to_owned(),
            )
        })?;
        self.charge(found.len())?;
        let mut hidden = Vec::new();
        for term in &found {
            let Term::Variant { negated, test } = term else {
                continue;
            };
            let MatchPattern::Variant(pattern) = &test.pattern.value else {
                return Err(SourceCheckError::Unsupported);
            };
            let named = pattern
                .fields
                .as_ref()
                .is_some_and(|fields| !fields.binders.value.is_empty());
            if named && (*negated || (connective == Connective::Or && found.len() > 1)) {
                return Err(self.explain(
                    SourceCheckError::Unsupported,
                    "an `is` test that names binders must be required by the condition: it may be joined with `and`, not `or`, and not negated"
                        .to_owned(),
                ));
            }
            if let Some(fields) = &pattern.fields {
                for binder in &fields.binders.value {
                    hidden.push(self.name(binder)?);
                }
            }
        }
        let mut scope = None;
        if !hidden.is_empty() {
            self.charge(self.proof.names.len())?;
            let saved = self.proof.names.clone();
            self.charge_product(self.proof.names.len(), hidden.len() + 1)?;
            self.proof.names.retain(|(name, _)| !hidden.contains(name));
            scope = Some((saved, hidden));
        }
        Ok(CondFrame {
            terms: found,
            connective,
            index: 0,
            operands: false,
            site,
            expected,
            scope,
        })
    }

    pub(super) fn cond_step(
        &mut self,
        mut frame: CondFrame<'a>,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        self.charge(2)?;
        let detached = matches!(&frame.site, Site::Guard { snapshot: None, .. });
        let truth = match frame.terms[frame.index] {
            Term::Relation(leaf) => {
                if !frame.operands {
                    frame.operands = true;
                    pending.push(EvalTask::Cond(Box::new(frame)));
                    pending.push(leaf.right.task());
                    pending.push(EvalTask::Expression(leaf.left, None, false));
                    return Ok(());
                }
                self.cond_relation(&frame, leaf, detached, values)?
            }
            Term::Variant { negated, test } => {
                if detached {
                    return Err(self.explain(
                        SourceCheckError::Unsupported,
                        "a guard that tests a variant is only supported on the arm that runs"
                            .to_owned(),
                    ));
                }
                let place = assign::split_place(&test.subject).is_some();
                if !frame.operands && !place {
                    frame.operands = true;
                    pending.push(EvalTask::Cond(Box::new(frame)));
                    pending.push(EvalTask::Expression(&test.subject, None, false));
                    return Ok(());
                }
                self.cond_variant(negated, test, place, values)?
            }
        };
        let last = frame.index + 1 == frame.terms.len();
        let continues = match frame.connective {
            Connective::And => truth,
            Connective::Or => !truth,
        };
        if continues && !last {
            frame.index += 1;
            frame.operands = false;
            pending.push(EvalTask::Cond(Box::new(frame)));
            return Ok(());
        }
        self.cond_finish(frame, truth, pending)
    }

    fn cond_relation(
        &mut self,
        frame: &CondFrame<'a>,
        leaf: Leaf<'a>,
        detached: bool,
        values: &mut Vec<Value>,
    ) -> Result<bool> {
        let relation = leaf.relation;
        let right = self.pop_operand(relation, values)?;
        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
        // An unrestricted domain compares as the number it holds.
        let left = self.condition_number(left)?;
        self.scalar(left)?;
        let left = self.ensure_identity(left)?;
        let holds = relation.possible(left, right);
        let fails = relation.negate().possible(left, right);
        if detached {
            // Only the body of an arm that is not selected is checked: its guard is taken to
            // hold, and what it proves is assumed when it can be.
            if holds {
                self.assume_relation(relation, left, right)?;
            }
            return Ok(true);
        }
        if frame.terms.len() == 1
            && !holds
            && let Site::Chain { expression, index } = &frame.site
            && let Expr::Chain(chain) = &expression.value
        {
            let arm = &chain.arms[*index];
            self.check_unreachable(
                arm.value.value.span_id,
                "this branch never runs: the condition is never true",
                |this| this.evaluate(&arm.value.value, frame.expected),
            )?;
        }
        let mut sides = Vec::new();
        if holds {
            sides.push(true);
        }
        if fails {
            sides.push(false);
        }
        let selected = self.choose(sides.len())?;
        if sides[selected] {
            self.assume_relation(relation, left, right)?;
        } else {
            self.assume_relation(relation.negate(), left, right)?;
        }
        Ok(sides[selected])
    }

    fn cond_variant(
        &mut self,
        negated: bool,
        test: &'a IsTest,
        place: bool,
        values: &mut Vec<Value>,
    ) -> Result<bool> {
        let MatchPattern::Variant(pattern) = &test.pattern.value else {
            return Err(SourceCheckError::Unsupported);
        };
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
            if matched {
                let linear = self.holds_linear(subject);
                self.bind_pattern(pattern, test.pattern.span_id, subject)?;
                if place
                    && let Expr::Name(name) = &test.subject.value
                    && linear
                    && let Some(index) = self.local_position(name)?
                {
                    self.proof.names[index].1.available = false;
                }
            }
        } else if !place && let Some(owner) = subject.owner {
            self.charge_flow()?;
            self.obligate(owner);
        }
        Ok(matched != negated)
    }

    fn cond_finish(
        &mut self,
        mut frame: CondFrame<'a>,
        truth: bool,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        let expected = frame.expected;
        // Where the site's value runs the binders are in scope, and they are dropped after it;
        // where it does not they were never available.
        if let Some((saved, hidden)) = frame.scope.take()
            && !matches!(frame.site, Site::Guard { .. })
        {
            if truth {
                pending.push(EvalTask::Scope(saved, hidden));
            } else {
                self.end_arm_scope(saved, &hidden)?;
            }
        }
        match frame.site {
            Site::Conditional { then, otherwise } => {
                let branch = if truth { then } else { otherwise };
                pending.push(EvalTask::Expression(branch, expected, false));
            }
            Site::Chain { expression, index } => {
                if truth {
                    let Expr::Chain(chain) = &expression.value else {
                        return Err(SourceCheckError::Malformed);
                    };
                    pending.push(EvalTask::Expression(&chain.arms[index].value.value, expected, false));
                } else {
                    pending.push(EvalTask::Chain(expression, index + 1, expected));
                }
            }
            Site::Guard {
                base,
                rest,
                subject,
                snapshot,
            } => {
                if !truth {
                    let snapshot = snapshot.ok_or(SourceCheckError::Malformed)?;
                    // The arm's body and its scope end are dropped, and the state before the
                    // arm returns for the next arm of the same member.
                    pending.truncate(base - 2);
                    self.proof.restore_validation(*snapshot);
                    self.candidate_tasks(rest, subject, expected, pending)?;
                }
            }
        }
        Ok(())
    }
}
