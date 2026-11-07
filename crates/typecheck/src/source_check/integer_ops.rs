//! `/ % & | ^ << >>` on exact integers: the domain each result ranges over.
//!
//! `+ - *` are computed and recorded (`eval.rs`); these seven are known to the checker only as
//! result domains, so a body that uses them is checked but not recorded for evaluation. Division
//! truncates toward zero and a remainder follows its dividend; `>>` keeps the sign. What would
//! not be defined is refused rather than guessed: a divisor that may be zero and a shift by a
//! negative or by too many bits are `Unproved`, a result outside the checker's width `Overflow`.
//! The guards that make them defined are the program's (`when rhs = 0 then return ...`).
use super::*;

const SHIFT_MAX: i32 = 255;

fn bit_length(value: I256) -> u32 {
    let magnitude = if value < I256::ZERO {
        value.checked_neg().unwrap_or(value)
    } else {
        value
    };
    256 - magnitude.leading_zeros()
}

/// Bits a number needs below its sign bit: a negative number needs as many as its complement.
fn magnitude_bits(value: I256) -> u32 {
    if value < I256::ZERO {
        bit_length(-value - I256::ONE)
    } else {
        bit_length(value)
    }
}

fn hull(values: &[I256]) -> (I256, I256) {
    let lower = values.iter().copied().fold(values[0], I256::min);
    let upper = values.iter().copied().fold(values[0], I256::max);
    (lower, upper)
}

impl<'a> Checker<'a> {
    pub(super) fn integer_operation(
        &mut self,
        operator: InfixOperator,
        left: Value,
        right: Value,
    ) -> Result<Value> {
        let left = self.scalar_facts(left)?;
        let right = self.scalar_facts(right)?;
        self.scalar(left)?;
        self.scalar(right)?;
        self.charge(8)?;
        let (l, r) = ((left.lower, left.upper), (right.lower, right.upper));
        match result_domain(operator, l, r) {
            Ok((lower, upper)) => Ok(Value::integer(lower, upper)),
            Err(DomainFailure::DivisorMayBeZero) => {
                let detail = "the divisor may be zero here".to_owned();
                Err(self.explain(SourceCheckError::Unproved, detail))
            }
            Err(DomainFailure::ShiftRange) => {
                let detail = format!("the shift must be from 0 to {SHIFT_MAX} bits here");
                Err(self.explain(SourceCheckError::Unproved, detail))
            }
            Err(DomainFailure::Overflow) => Err(SourceCheckError::Overflow),
            Err(DomainFailure::NotAnIntegerOperation) => Err(SourceCheckError::Unsupported),
        }
    }
}

/// Why an integer operation has no result domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DomainFailure {
    DivisorMayBeZero,
    ShiftRange,
    Overflow,
    NotAnIntegerOperation,
}

/// The domain `left <operator> right` ranges over, by the rules the checker proves with. The
/// single definition: the checker's `integer_operation`, `eval::interval` and the recorder (which
/// types a recorded operation) all call it.
pub(super) fn result_domain(
    operator: InfixOperator,
    l: (I256, I256),
    r: (I256, I256),
) -> std::result::Result<(I256, I256), DomainFailure> {
    Ok(match operator {
        InfixOperator::Divide | InfixOperator::Remainder => {
            if r.0 <= I256::ZERO && I256::ZERO <= r.1 {
                return Err(DomainFailure::DivisorMayBeZero);
            }
            if operator == InfixOperator::Divide {
                divide(l, r).map_err(|_| DomainFailure::Overflow)?
            } else {
                remainder(l, r)
            }
        }
        InfixOperator::BitAnd | InfixOperator::BitOr | InfixOperator::BitXor => bitwise(operator, l, r),
        InfixOperator::ShiftLeft | InfixOperator::ShiftRight => {
            if r.0 < I256::ZERO || r.1 > I256::from(SHIFT_MAX) {
                return Err(DomainFailure::ShiftRange);
            }
            shift(operator, l, r).map_err(|_| DomainFailure::Overflow)?
        }
        _ => return Err(DomainFailure::NotAnIntegerOperation),
    })
}

/// The domain a recorded operation is typed with. The operand types are the recorded ones, which
/// may be wider than the facts that held where the operation runs (a guard excluded a zero
/// divisor, a shift amount is bounded): a divisor is taken without zero, a shift amount within
/// the range the checker proves, so the type is sound for the paths the checker accepted.
pub(super) fn recorded_domain(
    operator: InfixOperator,
    l: (I256, I256),
    r: (I256, I256),
) -> Option<(I256, I256)> {
    match operator {
        InfixOperator::Divide | InfixOperator::Remainder => nonzero_divisor_domain(operator, l, r),
        InfixOperator::ShiftLeft | InfixOperator::ShiftRight => {
            let amounts = (r.0.max(I256::ZERO), r.1.min(I256::from(SHIFT_MAX)));
            (amounts.0 <= amounts.1).then(|| result_domain(operator, l, amounts).ok()).flatten()
        }
        _ => result_domain(operator, l, r).ok(),
    }
}

/// The domain of a division or remainder that is known to run only with a divisor that is not
/// zero, though its recorded type may include zero (a guard excluded it): the hull over the
/// negative and the positive parts of the divisor. `None` if the divisor can only be zero.
pub(super) fn nonzero_divisor_domain(
    operator: InfixOperator,
    l: (I256, I256),
    r: (I256, I256),
) -> Option<(I256, I256)> {
    let mut sides = Vec::new();
    if r.0 <= -I256::ONE {
        sides.push((r.0, r.1.min(-I256::ONE)));
    }
    if r.1 >= I256::ONE {
        sides.push((r.0.max(I256::ONE), r.1));
    }
    let mut hull: Option<(I256, I256)> = None;
    for side in sides {
        let (lower, upper) = result_domain(operator, l, side).ok()?;
        hull = Some(match hull {
            Some((a, b)) => (a.min(lower), b.max(upper)),
            None => (lower, upper),
        });
    }
    hull
}

fn divide(l: (I256, I256), r: (I256, I256)) -> Result<(I256, I256)> {
    let mut quotients = Vec::with_capacity(4);
    for x in [l.0, l.1] {
        for y in [r.0, r.1] {
            quotients.push(x.checked_div(y).ok_or(SourceCheckError::Overflow)?);
        }
    }
    Ok(hull(&quotients))
}

fn remainder(l: (I256, I256), r: (I256, I256)) -> (I256, I256) {
    if l.0 == l.1 && r.0 == r.1 {
        let exact = l.0.checked_rem(r.0).unwrap_or(I256::ZERO);
        return (exact, exact);
    }
    let widest = r.0.checked_abs().unwrap_or(r.0).max(r.1.checked_abs().unwrap_or(r.1));
    let below = widest - I256::ONE;
    let lower = if l.0 >= I256::ZERO { I256::ZERO } else { l.0.max(-below) };
    let upper = if l.1 <= I256::ZERO { I256::ZERO } else { l.1.min(below) };
    (lower, upper)
}

fn bitwise(operator: InfixOperator, l: (I256, I256), r: (I256, I256)) -> (I256, I256) {
    if l.0 == l.1 && r.0 == r.1 {
        let exact = match operator {
            InfixOperator::BitAnd => l.0 & r.0,
            InfixOperator::BitOr => l.0 | r.0,
            _ => l.0 ^ r.0,
        };
        return (exact, exact);
    }
    let (left_nonnegative, right_nonnegative) = (l.0 >= I256::ZERO, r.0 >= I256::ZERO);
    if left_nonnegative && right_nonnegative {
        let bits = bit_length(l.1.max(r.1));
        let mask = (I256::ONE << I256::from(bits)) - I256::ONE;
        return match operator {
            InfixOperator::BitAnd => (I256::ZERO, l.1.min(r.1)),
            InfixOperator::BitOr => (l.0.max(r.0), mask),
            _ => (I256::ZERO, mask),
        };
    }
    // An AND with a non-negative operand cannot exceed it, and is not negative.
    if operator == InfixOperator::BitAnd && (left_nonnegative || right_nonnegative) {
        let bound = if left_nonnegative && right_nonnegative {
            l.1.min(r.1)
        } else if left_nonnegative {
            l.1
        } else {
            r.1
        };
        return (I256::ZERO, bound);
    }
    // Otherwise the result fits in as many bits as the widest operand, sign included.
    let bits = [l.0, l.1, r.0, r.1].into_iter().map(magnitude_bits).max().unwrap_or(0);
    let edge = I256::ONE << I256::from(bits);
    (-edge, edge - I256::ONE)
}

fn shift(operator: InfixOperator, l: (I256, I256), r: (I256, I256)) -> Result<(I256, I256)> {
    let mut results = Vec::with_capacity(4);
    for x in [l.0, l.1] {
        for y in [r.0, r.1] {
            let amount = i32::try_from(y.as_i64()).map_err(|_| SourceCheckError::Overflow)?;
            results.push(if operator == InfixOperator::ShiftRight {
                x >> I256::from(amount)
            } else {
                let mut value = x;
                for _ in 0..amount {
                    value = value.checked_mul(I256::from(2)).ok_or(SourceCheckError::Overflow)?;
                }
                value
            });
        }
    }
    Ok(hull(&results))
}
