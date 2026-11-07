//! A guard that returns: `when comparison then <block ending in return>` at the start of a
//! function's body, with the rest of the body after it.
//!
//! Each guard is a replay choice with two outcomes: the guard holds and the path returns, or it
//! does not and the path goes on to the next statement. A path that returns ends there, so `n`
//! guards make `n + 1` worlds, not `2^n`. Facts follow the same split: the comparison is assumed
//! inside the block, and its negation holds for everything after the guard.
//!
//! A condition may join comparisons with one connective. `a and b` is a guard on `a` whose
//! success is a guard on `b`; `a or b` is a guard on `a` and, where `a` fails, a guard on `b`
//! that runs the same block. Either way every path carries exact facts and the cost is linear
//! in the number of comparisons. `not` and groups are seen through: the negation is pushed
//! inward (`not (a and b)` is `not a or not b`), so what is joined is always a list of
//! relations under one connective, and `x is 0` is `x = 0`. Mixing `and` with `or` after that
//! is not supported yet.
use super::*;
use aggregates::EvalTask;

/// How the comparisons of a condition combine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Connective {
    And,
    Or,
}

/// A relation between two numbers, with its negation. The facts can hold every one of these
/// but `NotEqual`, which only rules a path out when both sides are the same single number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Relation {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// `x is in D`: `x` lies in the domain's range, whose bounds are the two right operands.
    Within,
    /// `not x is in D`.
    Outside,
}

impl Relation {
    pub(super) fn from_binary(operator: BinaryOperator) -> Option<Self> {
        Some(match operator {
            BinaryOperator::Equal => Self::Equal,
            BinaryOperator::Less => Self::Less,
            BinaryOperator::LessOrEqual => Self::LessOrEqual,
            BinaryOperator::Greater => Self::Greater,
            BinaryOperator::GreaterOrEqual => Self::GreaterOrEqual,
            _ => return None,
        })
    }

    pub(super) fn negate(self) -> Self {
        match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::Less => Self::GreaterOrEqual,
            Self::LessOrEqual => Self::Greater,
            Self::Greater => Self::LessOrEqual,
            Self::GreaterOrEqual => Self::Less,
            Self::Within => Self::Outside,
            Self::Outside => Self::Within,
        }
    }

    /// The operator a fact can be recorded with, or `None` for `NotEqual`.
    pub(super) fn binary(self) -> Option<BinaryOperator> {
        Some(match self {
            Self::Equal => BinaryOperator::Equal,
            Self::NotEqual | Self::Within | Self::Outside => return None,
            Self::Less => BinaryOperator::Less,
            Self::LessOrEqual => BinaryOperator::LessOrEqual,
            Self::Greater => BinaryOperator::Greater,
            Self::GreaterOrEqual => BinaryOperator::GreaterOrEqual,
        })
    }

    /// Whether some numbers in these intervals stand in the relation.
    pub(super) fn possible(self, left: Value, right: Operand) -> bool {
        match (self, right) {
            (Self::Within, Operand::Range(low, high)) => {
                left.upper >= low.lower && left.lower <= high.upper
            }
            (Self::Outside, Operand::Range(low, high)) => {
                left.lower < low.lower || left.upper > high.upper
            }
            (_, Operand::One(right)) => match self.binary() {
                Some(operator) => Checker::comparison_possible(operator, left, right),
                None => {
                    !(left.lower == left.upper
                        && right.lower == right.upper
                        && left.lower == right.lower)
                }
            },
            _ => false,
        }
    }
}

/// What a relation is against: a number, or the two bounds of a domain.
#[derive(Clone, Copy)]
pub(super) enum Operand {
    One(Value),
    Range(Value, Value),
}

/// The right-hand side of a relation as written: an expression, or the domain after `is in`.
#[derive(Clone, Copy)]
pub(super) enum Side<'a> {
    Expr(&'a Expression),
    Domain(&'a Expression),
}

impl<'a> Side<'a> {
    pub(super) fn task(self) -> EvalTask<'a> {
        match self {
            Side::Expr(expression) => EvalTask::Expression(expression, None, false),
            Side::Domain(domain) => EvalTask::DomainBounds(domain),
        }
    }
}

/// One relation of a condition after negation is pushed in: `left <relation> right`.
#[derive(Clone, Copy)]
pub(super) struct Leaf<'a> {
    pub left: &'a Expression,
    pub right: Side<'a>,
    pub relation: Relation,
}

/// Where a guard has got to: which comparison, and whether its operands are evaluated.
#[derive(Clone, Copy)]
pub(super) struct GuardStage {
    pub leaf: usize,
    pub visited: bool,
}

/// The relations of a condition in source order, and how they combine. A single relation is
/// `And` of one. `None` for a condition this does not cover: a variant test, a mix of `and`
/// with `or`, anything that is not a comparison of numbers.
pub(super) fn conditions(condition: &Expression) -> Option<(Connective, Vec<Leaf<'_>>)> {
    let mut leaves = Vec::new();
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
                // Under a `not`, `and` becomes `or` and `or` becomes `and`.
                let this = match (operator.value, negated) {
                    (BinaryOperator::And, false) | (BinaryOperator::Or, true) => Connective::And,
                    _ => Connective::Or,
                };
                if *connective.get_or_insert(this) != this {
                    return None;
                }
                // The left operand is read first.
                stack.push((right, negated));
                stack.push((left, negated));
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                let relation = Relation::from_binary(operator.value)?;
                leaves.push(Leaf {
                    left,
                    right: Side::Expr(right),
                    relation: if negated { relation.negate() } else { relation },
                });
            }
            // `x is 5` on a number is `x = 5`; a variant test is not a relation of numbers.
            Expr::IsTest(test) => match &test.pattern.value {
                MatchPattern::Integer(number) => leaves.push(Leaf {
                    left: &test.subject,
                    right: Side::Expr(number),
                    relation: if negated {
                        Relation::NotEqual
                    } else {
                        Relation::Equal
                    },
                }),
                MatchPattern::Domain { domain, .. } => leaves.push(Leaf {
                    left: &test.subject,
                    right: Side::Domain(domain),
                    relation: if negated {
                        Relation::Outside
                    } else {
                        Relation::Within
                    },
                }),
                MatchPattern::Variant(_) | MatchPattern::Text(_) | MatchPattern::Else { .. } => {
                    return None;
                }
            },
            _ => return None,
        }
    }
    Some((connective.unwrap_or(Connective::And), leaves))
}

/// A condition that is one variant test, perhaps negated and parenthesized: `x is Active`,
/// `not x is Active`, `(x is Active)`. Whether it is negated, and the test.
pub(super) fn variant_test(condition: &Expression) -> Option<(bool, &IsTest)> {
    let mut negated = false;
    let mut next = condition;
    loop {
        match &next.value {
            Expr::Group(inner) => next = inner,
            Expr::Unary { operator, operand } if operator.value == UnaryOperator::Not => {
                negated = !negated;
                next = operand;
            }
            Expr::IsTest(test) if matches!(test.pattern.value, MatchPattern::Variant(_)) => {
                return Some((negated, test));
            }
            _ => return None,
        }
    }
}

/// Whether a block ends by returning, which makes the statement holding it a guard.
pub(super) fn returns(block: &Block) -> bool {
    matches!(
        block.statements.last().map(|last| &last.value),
        Some(Statement::Return(_))
    )
}

impl<'a> Checker<'a> {
    /// Every wrapper of a condition accounted for in the source: groups, `not`, `and`/`or`, the
    /// parts of a comparison or an `is` test. The operands themselves are evaluated by tasks.
    pub(super) fn condition_syntax(&mut self, condition: &'a Expression) -> Result<()> {
        let mut pending = vec![condition];
        while let Some(expression) = pending.pop() {
            self.charge(2)?;
            match &expression.value {
                Expr::Group(inner) => {
                    self.group_syntax(expression, inner)?;
                    pending.push(inner);
                }
                Expr::Unary { operator, operand } => {
                    self.keyword(operator.span_id, "not")?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, operand.span_id)?;
                    pending.push(operand);
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    if matches!(operator.value, BinaryOperator::And | BinaryOperator::Or) {
                        self.keyword(
                            operator.span_id,
                            if operator.value == BinaryOperator::And {
                                "and"
                            } else {
                                "or"
                            },
                        )?;
                        pending.extend([right.as_ref(), left.as_ref()]);
                    }
                }
                Expr::IsTest(test) => {
                    self.keyword(test.is_keyword, "is")?;
                    self.child(expression.span_id, test.subject.span_id)?;
                    self.child(expression.span_id, test.is_keyword)?;
                    self.child(expression.span_id, test.pattern.span_id)?;
                    if let MatchPattern::Domain {
                        in_keyword, domain, ..
                    } = &test.pattern.value
                    {
                        self.keyword(*in_keyword, "in")?;
                        self.child(test.pattern.span_id, *in_keyword)?;
                        self.child(test.pattern.span_id, domain.span_id)?;
                    }
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(())
    }

    /// Read the right operand of a relation from the value stack, ready to compare: a number
    /// with an identity, or a domain's two bounds. Pushed right first, so popped first.
    pub(super) fn pop_operand(
        &mut self,
        relation: Relation,
        values: &mut Vec<Value>,
    ) -> Result<Operand> {
        let mut take = |checker: &mut Self| -> Result<Value> {
            let value = values.pop().ok_or(SourceCheckError::Malformed)?;
            // An unrestricted domain compares as the number it holds (`offset < count` of two
            // `Length`s). A linear one is not read: that would take it apart.
            let value = checker.condition_number(value)?;
            checker.scalar(value)?;
            checker.ensure_identity(value)
        };
        if matches!(relation, Relation::Within | Relation::Outside) {
            let high = take(self)?;
            let low = take(self)?;
            Ok(Operand::Range(low, high))
        } else {
            Ok(Operand::One(take(self)?))
        }
    }

    /// The right operand of a relation, evaluated now rather than by tasks.
    pub(super) fn evaluate_operand(
        &mut self,
        side: Side<'a>,
        relation: Relation,
    ) -> Result<Operand> {
        let mut values = Vec::new();
        match side {
            Side::Expr(expression) => values.push(self.evaluate(expression, None)?),
            Side::Domain(domain) => self.domain_bounds_task(domain, &mut values)?,
        }
        self.pop_operand(relation, &mut values)
    }

    /// The bounds `is in` tests against, pushed lower then upper.
    pub(super) fn domain_bounds_task(
        &mut self,
        domain: &'a Expression,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        self.charge(4)?;
        let (lower, upper) = match &domain.value {
            // A written range, `is in -128 to 127`, with constant ends.
            Expr::Range { start, end, .. } => {
                let (low, high) = (self.expression(start)?, self.expression(end)?);
                if low.lower != low.upper || high.lower != high.upper {
                    return Err(SourceCheckError::Unsupported);
                }
                (low.lower, high.upper)
            }
            _ => match self.resolve_type(domain, &[])? {
                SourceType::Integer { lower, upper } => (lower, upper),
                other => self
                    .domain_bounds_of(other)
                    .ok_or(SourceCheckError::Type)?,
            },
        };
        values.push(Value::integer(lower, lower));
        values.push(Value::integer(upper, upper));
        Ok(())
    }

    /// Assume `left <relation> right` for the rest of the path. A not-equal and an outside leave
    /// no fact, and are refused only when no number could stand in them.
    pub(super) fn assume_relation(
        &mut self,
        relation: Relation,
        left: Value,
        right: Operand,
    ) -> Result<()> {
        self.scalar(left)?;
        if !relation.possible(left, right) {
            return Err(SourceCheckError::Type);
        }
        match (relation, right) {
            (Relation::Within, Operand::Range(low, high)) => {
                self.assume(BinaryOperator::GreaterOrEqual, left, low)?;
                self.assume(BinaryOperator::LessOrEqual, left, high)
            }
            (_, Operand::One(right)) => match relation.binary() {
                Some(operator) => self.assume(operator, left, right),
                None if relation == Relation::NotEqual => self.exclude_number(left, right),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    /// `left` is not the number `right`: a number can only be taken off an end of the range
    /// `left` is known to lie in (an interval has no hole), which is what a divisor needs after
    /// `when divisor = 0 then return ...` once a sign test has cut the range at zero.
    fn exclude_number(&mut self, left: Value, right: Value) -> Result<()> {
        self.charge(2)?;
        let (variable, number) = if right.lower == right.upper {
            (left, right.lower)
        } else if left.lower == left.upper {
            (right, left.lower)
        } else {
            return Ok(());
        };
        let Some(identity) = variable.identity else {
            return Ok(());
        };
        // Identities are handed out in order, so the scalars are sorted by them.
        self.charge((usize::BITS - self.proof.scalars.len().leading_zeros()) as usize + 1)?;
        if let Ok(position) = self
            .proof
            .scalars
            .binary_search_by_key(&identity, |(candidate, _, _)| *candidate)
        {
            let entry = &mut self.proof.scalars[position];
            if entry.1 == number && entry.1 < entry.2 {
                entry.1 = number.checked_add(I256::ONE).ok_or(SourceCheckError::Type)?;
            } else if entry.2 == number && entry.1 < entry.2 {
                entry.2 = number.checked_sub(I256::ONE).ok_or(SourceCheckError::Type)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn guard_tasks(
        &mut self,
        when: &'a WhenStatement,
        span: SpanId,
        expected: Option<SourceType>,
        base: usize,
        stage: GuardStage,
        pending: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        if when.else_branch.is_some() || !returns(&when.then_block.value) {
            return Err(SourceCheckError::Unsupported);
        }
        let (connective, leaves) =
            conditions(&when.condition).ok_or(SourceCheckError::Unsupported)?;
        let leaf = leaves[stage.leaf];
        if !stage.visited {
            if stage.leaf == 0 {
                self.keyword(when.when_keyword, "when")?;
                self.keyword(when.then_keyword, "then")?;
                self.child(span, when.condition.span_id)?;
                self.child(span, when.then_block.span_id)?;
                self.condition_syntax(&when.condition)?;
            }
            self.charge(2)?;
            let visited = GuardStage {
                leaf: stage.leaf,
                visited: true,
            };
            pending.push(EvalTask::Guard(when, span, expected, base, visited));
            pending.push(leaf.right.task());
            pending.push(EvalTask::Expression(leaf.left, None, false));
            return Ok(());
        }
        let relation = leaf.relation;
        let right = self.pop_operand(relation, values)?;
        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
        let left = self.condition_number(left)?;
        self.scalar(left)?;
        let left = self.ensure_identity(left)?;
        let holds = relation.possible(left, right);
        let fails = relation.negate().possible(left, right);
        let mut paths = Vec::new();
        if holds {
            paths.push(true);
        }
        if fails {
            paths.push(false);
        }
        // An infeasible side is not checked in this world, as with the value conditional.
        let selected = self.choose(paths.len())?;
        let last = stage.leaf + 1 == leaves.len();
        let next = GuardStage {
            leaf: stage.leaf + 1,
            visited: false,
        };
        if paths[selected] {
            self.assume_relation(relation, left, right)?;
            match (connective, last) {
                // `and`: the next comparison decides, and if it fails the rest of the body runs.
                (Connective::And, false) => {
                    pending.push(EvalTask::Guard(when, span, expected, base, next));
                }
                // The whole condition holds, or one of an `or` does: the path returns.
                _ => {
                    pending.truncate(base);
                    if self.proof.loop_depth > 0 {
                        pending.push(EvalTask::Returned);
                    }
                    self.statement_tasks(
                        when.then_block.span_id,
                        &when.then_block.value.statements,
                        expected,
                        base,
                        pending,
                    )?;
                }
            }
        } else {
            self.assume_relation(relation.negate(), left, right)?;
            // `or`: this comparison failed, so the next one decides; `and` and the last `or`
            // comparison fall through to the rest of the body.
            if connective == Connective::Or && !last {
                pending.push(EvalTask::Guard(when, span, expected, base, next));
            }
        }
        Ok(())
    }
}

impl<'a> Checker<'a> {
    /// A value in a condition: an unrestricted domain is the number it holds; anything else is
    /// itself (and a linear domain is refused as before, since reading it would take it apart).
    pub(super) fn condition_number(&mut self, value: Value) -> Result<Value> {
        let unrestricted = value.owner.is_some_and(|owner| self.proof.owners[owner].unrestricted);
        if unrestricted && self.domain_bounds_of(value.ty).is_some() {
            // The number the domain holds, with what an earlier condition of this world taught
            // about it (a path that chose `first = 224` cannot then choose `first = 237`).
            let number = self.domain_number(value)?;
            return self.scalar_facts(number);
        }
        Ok(value)
    }
}
