//! Compile-time evaluation of recorded bodies.
//!
//! The rule this module serves: when every input of an operation is known at compile
//! time, the operation is evaluated at compile time. A recorded body is pure data, so
//! it can be run here with no machine and no code generation; a consumer that holds the
//! constant arguments of a call gets the call's value, not a call.
//!
//! The arithmetic in this file is the single definition of `+`, `-` and `*` on exact
//! integers: the checker derives result domains with [`interval`], recording and this
//! evaluator compute values with [`apply`], and a lowering never has to guess.

use super::SourceType;
use super::body::{
    CheckedArithmetic, CheckedSourceBody, CheckedSourceBodyKind, CheckedSourceCallee,
    CheckedSourceLoopCarry, InfixOperator,
};
use i256::I256;

/// A value the evaluator can hold: an exact integer, or a record of values in schema
/// field order. Records are what Gin machine models are made of, so they are evaluated
/// like any other data. A record names its schema, which is what tells the members of a
/// sum apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evaluated {
    Integer(I256),
    /// Compile-time text.
    Text(String),
    Record {
        schema: u32,
        fields: Vec<Evaluated>,
    },
}

/// Bodies the evaluator may enter when a body calls another function.
pub trait BodyResolver {
    /// `None` when the callee has no recorded body here (for example a function of a
    /// dependency, whose capsule is body-free). Evaluation then stops, never guesses.
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody>;
}

/// Calls the checker's fold follows: nested this deep, evaluation is iterative within one body.
/// A parameter of [`evaluate_value_with`], not a property of the evaluator.
pub const FOLD_DEPTH: usize = 32;

/// `left <operator> right`, or `None` when the exact result does not fit the checker's
/// integer width. Never wraps.
pub fn apply(operator: CheckedArithmetic, left: I256, right: I256) -> Option<I256> {
    match operator {
        CheckedArithmetic::Add => left.checked_add(right),
        CheckedArithmetic::Subtract => left.checked_sub(right),
        CheckedArithmetic::Multiply => left.checked_mul(right),
        other => match exact_binary(other.binary(), left, right) {
            Exact::Value(value) => Some(value),
            _ => None,
        },
    }
}

/// What exact integer arithmetic on two constants gives: the value, or why there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exact {
    Value(I256),
    /// The exact result does not fit the checker's integer width, or the divisor is zero.
    Overflow,
    /// A shift by a negative amount or by 255 bits or more has no exact meaning.
    ShiftAmount(I256),
    /// Not an arithmetic operator (a comparison, `and`, `or`).
    NotArithmetic,
}

/// The single definition of exact integer arithmetic on constants: `+ - *` and the bit operations
/// (two's complement); `/` and
/// `%` truncate toward zero and a remainder follows its dividend; `<<` is a multiplication by a
/// power of two and `>>` keeps the sign. Never wraps. The constants evaluator and every other
/// consumer of constant arithmetic call this rather than repeat it.
pub fn exact_binary(operator: ast::BinaryOperator, left: I256, right: I256) -> Exact {
    use ast::BinaryOperator as B;
    let result = match operator {
        B::Add => left.checked_add(right),
        B::Subtract => left.checked_sub(right),
        B::Multiply => left.checked_mul(right),
        B::BitAnd => Some(left & right),
        B::BitOr => Some(left | right),
        B::BitXor => Some(left ^ right),
        B::Divide => left.checked_div(right),
        B::Remainder => left.checked_rem(right),
        B::ShiftLeft | B::ShiftRight => {
            if !(right >= I256::ZERO && right < I256::from(255_u32)) {
                return Exact::ShiftAmount(right);
            }
            let amount = right.as_i64() as u32;
            if operator == B::ShiftLeft {
                (I256::ONE << amount).checked_mul(left)
            } else {
                Some(left >> amount)
            }
        }
        _ => return Exact::NotArithmetic,
    };
    result.map_or(Exact::Overflow, Exact::Value)
}

/// Whether `left <comparison> right` holds, or `None` for an operator that is not a comparison of
/// integers. The single definition of the comparisons a `Select` and a `while` test.
pub fn compare(comparison: InfixOperator, left: I256, right: I256) -> Option<bool> {
    Some(match comparison {
        InfixOperator::Equal => left == right,
        InfixOperator::Less => left < right,
        InfixOperator::LessOrEqual => left <= right,
        InfixOperator::Greater => left > right,
        InfixOperator::GreaterOrEqual => left >= right,
        _ => return None,
    })
}

/// `value` reduced into `lower..=upper`: the value congruent to it modulo the domain's
/// width. `None` when the width itself does not fit the checker's integer width.
pub fn wrap_into(value: I256, lower: I256, upper: I256) -> Option<I256> {
    let width = upper.checked_sub(lower)?.checked_add(I256::ONE)?;
    let offset = value.checked_sub(lower)?;
    let remainder = offset.checked_rem(width)?;
    let remainder = if remainder < I256::ZERO {
        remainder.checked_add(width)?
    } else {
        remainder
    };
    remainder.checked_add(lower)
}

/// The exact result domain of `operator` over two domains. Each operation is monotone or
/// bilinear in its operands, so the extremes lie on the corners.
pub fn interval(
    operator: CheckedArithmetic,
    left: (I256, I256),
    right: (I256, I256),
) -> Option<(I256, I256)> {
    use super::integer_ops::result_domain;
    let integer_operation = match operator {
        CheckedArithmetic::Divide => Some(InfixOperator::Divide),
        CheckedArithmetic::Remainder => Some(InfixOperator::Remainder),
        CheckedArithmetic::BitAnd => Some(InfixOperator::BitAnd),
        CheckedArithmetic::BitOr => Some(InfixOperator::BitOr),
        CheckedArithmetic::BitXor => Some(InfixOperator::BitXor),
        CheckedArithmetic::ShiftLeft => Some(InfixOperator::ShiftLeft),
        CheckedArithmetic::ShiftRight => Some(InfixOperator::ShiftRight),
        _ => None,
    };
    // Division and the bit operations are not monotone in their corners: the checker's own rules.
    if let Some(integer_operation) = integer_operation {
        return result_domain(integer_operation, left, right).ok();
    }
    let mut bounds: Option<(I256, I256)> = None;
    for l in [left.0, left.1] {
        for r in [right.0, right.1] {
            let value = apply(operator, l, r)?;
            bounds = Some(match bounds {
                Some((lower, upper)) => (lower.min(value), upper.max(value)),
                None => (value, value),
            });
        }
    }
    bounds
}

/// The integer a body returns for these integer arguments, or `None` when it cannot be
/// known here. `fuel` bounds the total number of nodes evaluated, including callees, so a
/// pathological body costs a bounded amount of compile time and then stays a call.
pub fn evaluate_body(
    body: &CheckedSourceBody,
    arguments: &[I256],
    resolver: &dyn BodyResolver,
    fuel: &mut u64,
) -> Option<I256> {
    let arguments: Vec<Evaluated> = arguments.iter().copied().map(Evaluated::Integer).collect();
    match evaluate_value(body, &arguments, resolver, fuel)? {
        Evaluated::Integer(value) => Some(value),
        Evaluated::Record { .. } | Evaluated::Text(_) => None,
    }
}

/// [`evaluate_body`] for arguments and a result of any evaluable kind.
pub fn evaluate_value(
    body: &CheckedSourceBody,
    arguments: &[Evaluated],
    resolver: &dyn BodyResolver,
    fuel: &mut u64,
) -> Option<Evaluated> {
    evaluate_value_with(body, arguments, resolver, fuel, FOLD_DEPTH)
}

/// [`evaluate_value`] with the call depth the caller allows. The budget is the caller's: one unit
/// (a node evaluated, or a loop pass) and one depth, supplied per use. The checker's fold passes
/// `FOLD_FUEL` and [`FOLD_DEPTH`].
pub fn evaluate_value_with(
    body: &CheckedSourceBody,
    arguments: &[Evaluated],
    resolver: &dyn BodyResolver,
    fuel: &mut u64,
    depth_max: usize,
) -> Option<Evaluated> {
    run_body(body, arguments, resolver, fuel, (0, depth_max))?.0
}

/// The result of a body, and the value each `mut` parameter holds when it returns.
type Outcome = (Option<Evaluated>, Vec<Option<Evaluated>>);

fn run_body(
    body: &CheckedSourceBody,
    arguments: &[Evaluated],
    resolver: &dyn BodyResolver,
    fuel: &mut u64,
    (depth, depth_max): (usize, usize),
) -> Option<Outcome> {
    if depth > depth_max || body.effects {
        return None;
    }
    // The arena is postorder, so one forward pass sees every operand before its user. The
    // nodes of a loop's body come before the loop and are run by it, once per pass.
    let mut run = Run {
        body,
        arguments,
        resolver,
        depth,
        depth_max,
        values: vec![None; body.expressions.len()],
        finals: vec![None; body.expressions.len()],
        outs: vec![None; body.expressions.len()],
        nesting: nesting(body),
    };
    for id in 0..body.expressions.len() {
        if run.nesting[id] == 0 {
            run.step(id, fuel)?;
        }
    }
    let outputs = body
        .outputs
        .iter()
        .map(|node| run.values.get(*node as usize).cloned().flatten())
        .collect();
    Some((returned(body, &run.values), outputs))
}

/// How many loop bodies each node is inside.
fn nesting(body: &CheckedSourceBody) -> Vec<u32> {
    let mut nesting = vec![0; body.expressions.len()];
    for (id, expression) in body.expressions.iter().enumerate() {
        if let CheckedSourceBodyKind::Loop { body_start, .. }
        | CheckedSourceBodyKind::While { body_start, .. } = &expression.kind
        {
            for inside in *body_start as usize..id {
                nesting[inside] += 1;
            }
        }
    }
    nesting
}

/// One evaluation of a body: the value of every node so far, and what each loop left.
struct Run<'a> {
    body: &'a CheckedSourceBody,
    arguments: &'a [Evaluated],
    resolver: &'a dyn BodyResolver,
    depth: usize,
    depth_max: usize,
    values: Vec<Option<Evaluated>>,
    /// For a `Loop` node, the carried values after its last pass.
    finals: Vec<Option<Vec<Option<Evaluated>>>>,
    /// For a `Call` node, what each `mut` parameter of the callee held when it returned.
    outs: Vec<Option<Vec<Option<Evaluated>>>>,
    nesting: Vec<u32>,
}

impl Run<'_> {
    /// Evaluate node `id`. `None` stops the whole evaluation: something it needs is unknown.
    fn step(&mut self, id: usize, fuel: &mut u64) -> Option<()> {
        *fuel = fuel.checked_sub(1)?;
        let body = self.body;
        let expression = &body.expressions[id];
        let arguments = self.arguments;
        let resolver = self.resolver;
        let depth = self.depth;
        if let CheckedSourceBodyKind::Loop {
            start,
            end,
            counter,
            body_start,
            carried,
        } = &expression.kind
        {
            return self.run_loop(id, (*start, *end, *counter, *body_start), carried, fuel);
        }
        if let CheckedSourceBodyKind::Call { callee, arguments: ids, .. } = &expression.kind {
            let known: Option<Vec<Evaluated>> = ids
                .iter()
                .map(|id| known(&self.values, *id).cloned())
                .collect();
            let outcome = match (known, resolver.body(*callee)) {
                (Some(known), Some(target)) => {
                    run_body(target, &known, resolver, fuel, (depth + 1, self.depth_max))
                }
                _ => None,
            };
            let (value, outs) = match outcome {
                Some((value, outs)) => (value, Some(outs)),
                None => (None, None),
            };
            self.values[id] = value;
            self.outs[id] = outs;
            return Some(());
        }
        let values = &self.values;
        let value = match &expression.kind {
            CheckedSourceBodyKind::Integer(value) => Some(Evaluated::Integer(*value)),
            CheckedSourceBodyKind::Text(text) => Some(Evaluated::Text(text.clone())),
            CheckedSourceBodyKind::Parameter { index } => arguments
                .get(*index as usize)
                .cloned()
                .filter(|value| within(expression.ty, value)),
            CheckedSourceBodyKind::Binary {
                operator,
                left,
                right,
            } => match (known(&values, *left)?, known(&values, *right)?) {
                (Evaluated::Integer(left), Evaluated::Integer(right)) => {
                    apply(*operator, *left, *right)
                        .map(Evaluated::Integer)
                        .filter(|value| within(expression.ty, value))
                }
                _ => None,
            },
            CheckedSourceBodyKind::Wrap { operand } => {
                match (known(&values, *operand)?, expression.ty) {
                    (Evaluated::Integer(value), SourceType::Integer { lower, upper }) => {
                        wrap_into(*value, lower, upper).map(Evaluated::Integer)
                    }
                    _ => None,
                }
            }
            CheckedSourceBodyKind::Construct { schema, fields } => fields
                .iter()
                .map(|id| known(&values, *id).cloned())
                .collect::<Option<Vec<_>>>()
                .map(|fields| Evaluated::Record {
                    schema: *schema,
                    fields,
                }),
            CheckedSourceBodyKind::Undefined => None,
            CheckedSourceBodyKind::Bundle { parts } => parts
                .iter()
                .map(|id| known(&values, *id).cloned())
                .collect::<Option<Vec<_>>>()
                .map(|fields| Evaluated::Record { schema: u32::MAX, fields }),
            CheckedSourceBodyKind::Part { bundle, index } => match known(&values, *bundle)? {
                Evaluated::Record { fields, .. } => fields.get(*index as usize).cloned(),
                Evaluated::Integer(_) | Evaluated::Text(_) => None,
            },
            CheckedSourceBodyKind::Field { record, index } => match known(&values, *record)? {
                Evaluated::Record { fields, .. } => fields.get(*index as usize).cloned(),
                Evaluated::Integer(_) | Evaluated::Text(_) => None,
            },
            CheckedSourceBodyKind::VariantField { subject, index, .. } => match known(&values, *subject)? {
                // An arm the subject does not take reads whatever sits at that position; the
                // value is meaningless and never selected, as for `Field`.
                Evaluated::Record { fields, .. } => {
                    fields.get(*index as usize).cloned()
                }
                Evaluated::Integer(_) | Evaluated::Text(_) => None,
            },
            // Every arm was evaluated above; the subject's member picks one. An arm the
            // subject does not take may hold a meaningless value, and is never read.
            CheckedSourceBodyKind::Match { subject, arms, .. } => match known(&values, *subject)? {
                Evaluated::Record { schema, .. } => arms
                    .iter()
                    .find(|arm| arm.schema == *schema)
                    .and_then(|arm| known(&values, arm.body).cloned()),
                Evaluated::Integer(_) | Evaluated::Text(_) => None,
            },
            CheckedSourceBodyKind::Select {
                comparison,
                left,
                right,
                then_branch,
                else_branch,
                ..
            } => match (known(&values, *left)?, known(&values, *right)?) {
                (Evaluated::Integer(left), Evaluated::Integer(right)) => {
                    let holds = compare(*comparison, *left, *right)?;
                    let chosen = if holds { then_branch } else { else_branch };
                    known(&values, *chosen).cloned()
                }
                _ => None,
            },
            CheckedSourceBodyKind::Call { .. } => unreachable!("handled above"),
            // Which function a bound reaches depends on the type given for it: not known here.
            CheckedSourceBodyKind::BoundCall { .. } => None,
            // A size that depends on a type parameter is not known here.
            CheckedSourceBodyKind::Layout { .. } => None,
            // Bytes in memory are not a compile-time value.
            CheckedSourceBodyKind::TextBytes { .. } | CheckedSourceBodyKind::SubArray { .. } | CheckedSourceBodyKind::RepeatArray { .. } => None,
            CheckedSourceBodyKind::CallOut { call, index } => self
                .outs
                .get(*call as usize)?
                .as_ref()?
                .get(*index as usize)?
                .clone(),
            CheckedSourceBodyKind::Widen { operand } => known(values, *operand).cloned(),
            CheckedSourceBodyKind::LoopCounter | CheckedSourceBodyKind::LoopCarried => {
                // Set by the loop that runs this pass.
                values.get(id)?.clone()
            }
            CheckedSourceBodyKind::LoopResult { looped, index } => self
                .finals
                .get(*looped as usize)?
                .as_ref()?
                .get(*index as usize)?
                .clone(),
            CheckedSourceBodyKind::Loop { .. } => unreachable!("handled above"),
            // A `while` is not folded: how many passes it runs is not worked out here, so
            // what it leaves is unknown and anything that reads it is computed at run time.
            CheckedSourceBodyKind::While { .. } => None,
            CheckedSourceBodyKind::Unit
            | CheckedSourceBodyKind::Hole
            | CheckedSourceBodyKind::Instruction { .. }
            | CheckedSourceBodyKind::Load { .. }
            | CheckedSourceBodyKind::Store { .. }
            | CheckedSourceBodyKind::Return { .. }
            | CheckedSourceBodyKind::Block { .. } => None,
        };
        self.values[id] = value;
        Some(())
    }

    /// Run the passes of a loop: its body nodes, in order, once per number from `start` to
    /// `end`, each pass seeing the values the one before carried.
    fn run_loop(
        &mut self,
        id: usize,
        (start, end, counter, body_start): (u32, u32, u32, u32),
        carried: &[CheckedSourceLoopCarry],
        fuel: &mut u64,
    ) -> Option<()> {
        let (Evaluated::Integer(first), Evaluated::Integer(last)) = (
            known(&self.values, start)?.clone(),
            known(&self.values, end)?.clone(),
        ) else {
            return None;
        };
        let mut carry = carried
            .iter()
            .map(|carry| known(&self.values, carry.initial).cloned())
            .collect::<Vec<_>>();
        let inside = self.nesting[id] + 1;
        let mut number = first;
        while number <= last {
            *fuel = fuel.checked_sub(1)?;
            self.values[counter as usize] = Some(Evaluated::Integer(number));
            for (slot, value) in carried.iter().zip(&carry) {
                self.values[slot.placeholder as usize] = value.clone();
            }
            for node in body_start as usize..id {
                if self.nesting[node] == inside {
                    self.step(node, fuel)?;
                }
            }
            carry = carried
                .iter()
                .map(|slot| known(&self.values, slot.next).cloned())
                .collect();
            number = number.checked_add(I256::ONE)?;
        }
        self.finals[id] = Some(carry);
        Some(())
    }
}

fn known(values: &[Option<Evaluated>], id: u32) -> Option<&Evaluated> {
    values.get(id as usize)?.as_ref()
}

/// The value a body returns: the block's `result` when that is a `return`. The first `return` among
/// the statements is the same node for every recorded body (a guard that returns is folded into a
/// selection, so a block has one); that is asserted in debug builds and tested over the corpus.
fn returned(body: &CheckedSourceBody, values: &[Option<Evaluated>]) -> Option<Evaluated> {
    let root = body.expressions.get(body.root as usize)?;
    let CheckedSourceBodyKind::Block { statements, result } = &root.kind else {
        // A body written as one expression returns that expression's value.
        return values.get(body.root as usize).cloned().flatten();
    };
    let first = statements.iter().find_map(|statement| match body.expressions.get(*statement as usize)?.kind {
        CheckedSourceBodyKind::Return { value } => Some(value),
        _ => None,
    });
    if let Some(CheckedSourceBodyKind::Return { value }) = body.expressions.get(*result as usize).map(|e| &e.kind) {
        debug_assert!(first.is_none_or(|first| first == *value), "the block's result is not its first return");
        return values.get(*value as usize).cloned().flatten();
    }
    // A block whose result is not a return: the first one among the statements, as before.
    values.get(first? as usize).cloned().flatten()
}

fn within(ty: SourceType, value: &Evaluated) -> bool {
    match (ty, value) {
        (SourceType::Integer { lower, upper }, Evaluated::Integer(value)) => {
            lower <= *value && *value <= upper
        }
        (SourceType::Aggregate { .. } | SourceType::Sum { .. }, Evaluated::Record { .. }) => true,
        (SourceType::Text, Evaluated::Text(_)) => true,
        _ => false,
    }
}
