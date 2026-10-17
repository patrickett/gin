use ast::integer::{
    CanonicalPredicate, ContainmentProof, FiniteHull, IntegerDomain, IntegerValidity,
};
use ast::{ConstValue, Ty};
use diagnostic::Diagnostic;
use diagnostic::code::IntegerCode;
use i256::I256;

use crate::intrinsic::IntrinsicOp;

pub(crate) fn fold(
    op: IntrinsicOp,
    values: &[ConstValue],
    operands: [&Ty; 2],
) -> Option<Result<ConstValue, Diagnostic>> {
    if !operands
        .iter()
        .all(|ty| ty.anonymous_integer_validity().is_some())
    {
        return None;
    }
    let [ConstValue::Int(left), ConstValue::Int(right)] = values else {
        return None;
    };
    let result = match op {
        IntrinsicOp::BitsAdd => left.checked_add(*right),
        IntrinsicOp::BitsSubtract => left.checked_sub(*right),
        IntrinsicOp::BitsMultiply => left.checked_mul(*right),
        IntrinsicOp::BitsShiftLeft => {
            if *right < I256::from(0) || *right > I256::from(255) {
                return Some(Err(valid_shift_count_required()));
            }
            checked_left_shift(*left, right.as_u32())
        }
        IntrinsicOp::BitsShiftRightArithmetic | IntrinsicOp::BitsShiftRightLogical => {
            if *right < I256::from(0) || *right > I256::from(255) {
                return Some(Err(valid_shift_count_required()));
            }
            Some(*left >> right.as_u32())
        }
        IntrinsicOp::BitsSignedDivide | IntrinsicOp::BitsUnsignedDivide => {
            if *right == I256::from(0) {
                return Some(Err(nonzero_divisor_required()));
            }
            left.checked_div(*right)
        }
        IntrinsicOp::BitsSignedRemainder | IntrinsicOp::BitsUnsignedRemainder => {
            if *right == I256::from(0) {
                return Some(Err(nonzero_divisor_required()));
            }
            if *right == I256::from(-1) {
                Some(I256::from(0))
            } else {
                left.checked_rem(*right)
            }
        }
        _ => return None,
    };
    Some(result.map(ConstValue::Int).ok_or_else(overflow))
}

pub(crate) fn result_type(
    op: IntrinsicOp,
    operands: [&Ty; 2],
    values: [Option<&ConstValue>; 2],
) -> Option<Result<Ty, Diagnostic>> {
    if !matches!(
        op,
        IntrinsicOp::BitsAdd
            | IntrinsicOp::BitsSubtract
            | IntrinsicOp::BitsMultiply
            | IntrinsicOp::BitsShiftLeft
            | IntrinsicOp::BitsShiftRightArithmetic
            | IntrinsicOp::BitsShiftRightLogical
            | IntrinsicOp::BitsSignedDivide
            | IntrinsicOp::BitsUnsignedDivide
            | IntrinsicOp::BitsSignedRemainder
            | IntrinsicOp::BitsUnsignedRemainder
    ) {
        return None;
    }
    let left = operand_domain(operands[0], values[0])?;
    let right = operand_domain(operands[1], values[1])?;
    if matches!(
        op,
        IntrinsicOp::BitsShiftLeft
            | IntrinsicOp::BitsShiftRightArithmetic
            | IntrinsicOp::BitsShiftRightLogical
    ) && right.prove_containment(&IntegerDomain::bounded(0.into(), 255.into()).unwrap())
        != ContainmentProof::Proven
    {
        return Some(Err(valid_shift_count_required()));
    }
    if matches!(
        op,
        IntrinsicOp::BitsSignedDivide
            | IntrinsicOp::BitsUnsignedDivide
            | IntrinsicOp::BitsSignedRemainder
            | IntrinsicOp::BitsUnsignedRemainder
    ) && right.membership(I256::from(0)) != ContainmentProof::Disproven
    {
        return Some(Err(nonzero_divisor_required()));
    }
    if left.storage_hull().is_none() || right.storage_hull().is_none() {
        let nonnegative = IntegerDomain::from_refinement(&ast::PredicateExpr::Ge(0_i128.into()));
        let domain = if matches!(op, IntrinsicOp::BitsAdd | IntrinsicOp::BitsMultiply)
            && left.prove_containment(&nonnegative) == ContainmentProof::Proven
            && right.prove_containment(&nonnegative) == ContainmentProof::Proven
        {
            nonnegative
        } else {
            IntegerDomain::unbounded()
        };
        return Some(Ok(Ty::AnonymousInteger {
            validity: IntegerValidity::new(domain),
        }));
    }
    let bounds =
        bounded_hull(&left, I256::MIN, I256::MAX).zip(bounded_hull(&right, I256::MIN, I256::MAX));
    let Some((left_hull, right_hull)) = bounds else {
        return Some(Ok(Ty::AnonymousInteger {
            validity: IntegerValidity::new(IntegerDomain::empty()),
        }));
    };
    Some(
        result_bounds(op, &left_hull, &right_hull, &right).map(|(min, max)| Ty::AnonymousInteger {
            validity: IntegerValidity::new(
                IntegerDomain::bounded(min, max).expect("ordered bounds"),
            ),
        }),
    )
}

fn bounded_hull(domain: &IntegerDomain, min: I256, max: I256) -> Option<FiniteHull> {
    let hull = domain.storage_hull()?;
    let mut min = hull.min().max(min);
    let mut max = hull.max().min(max);
    if let CanonicalPredicate::HullExcluding { excluded, .. } = domain.predicate() {
        while min <= max && excluded.binary_search(&min).is_ok() {
            min = min.checked_add(I256::from(1))?;
        }
        while min <= max && excluded.binary_search(&max).is_ok() {
            max = max.checked_sub(I256::from(1))?;
        }
    }
    FiniteHull::new(min, max)
}

fn operand_domain(ty: &Ty, value: Option<&ConstValue>) -> Option<IntegerDomain> {
    let validity = ty.anonymous_integer_validity()?;
    match value {
        Some(ConstValue::Int(value)) => IntegerDomain::bounded(*value, *value),
        _ => Some(validity.domain().clone()),
    }
}

fn result_bounds(
    op: IntrinsicOp,
    left: &FiniteHull,
    right: &FiniteHull,
    right_domain: &IntegerDomain,
) -> Result<(I256, I256), Diagnostic> {
    let bounds = match op {
        IntrinsicOp::BitsAdd => left
            .min()
            .checked_add(right.min())
            .zip(left.max().checked_add(right.max())),
        IntrinsicOp::BitsSubtract => left
            .min()
            .checked_sub(right.max())
            .zip(left.max().checked_sub(right.min())),
        IntrinsicOp::BitsMultiply => {
            let products = [
                left.min().checked_mul(right.min()),
                left.min().checked_mul(right.max()),
                left.max().checked_mul(right.min()),
                left.max().checked_mul(right.max()),
            ];
            let [
                Some(low_low),
                Some(low_high),
                Some(high_low),
                Some(high_high),
            ] = products
            else {
                return Err(overflow());
            };
            Some((
                low_low.min(low_high).min(high_low).min(high_high),
                low_low.max(low_high).max(high_low).max(high_high),
            ))
        }
        IntrinsicOp::BitsShiftLeft => {
            let shifts = [
                checked_left_shift(left.min(), right.min().as_u32()),
                checked_left_shift(left.min(), right.max().as_u32()),
                checked_left_shift(left.max(), right.min().as_u32()),
                checked_left_shift(left.max(), right.max().as_u32()),
            ];
            let [
                Some(low_low),
                Some(low_high),
                Some(high_low),
                Some(high_high),
            ] = shifts
            else {
                return Err(overflow());
            };
            Some((
                low_low.min(low_high).min(high_low).min(high_high),
                low_low.max(low_high).max(high_low).max(high_high),
            ))
        }
        IntrinsicOp::BitsShiftRightArithmetic | IntrinsicOp::BitsShiftRightLogical => {
            let low_low = left.min() >> right.min().as_u32();
            let low_high = left.min() >> right.max().as_u32();
            let high_low = left.max() >> right.min().as_u32();
            let high_high = left.max() >> right.max().as_u32();
            Some((
                low_low.min(low_high).min(high_low).min(high_high),
                low_low.max(low_high).max(high_low).max(high_high),
            ))
        }
        IntrinsicOp::BitsSignedDivide | IntrinsicOp::BitsUnsignedDivide => {
            return division_bounds(left, right_domain);
        }
        IntrinsicOp::BitsSignedRemainder | IntrinsicOp::BitsUnsignedRemainder => {
            return Ok(remainder_bounds(left, right));
        }
        _ => unreachable!("bounded anonymous arithmetic"),
    };
    bounds.ok_or_else(overflow)
}

fn checked_left_shift(value: I256, count: u32) -> Option<I256> {
    assert!(count < 256);
    if value == I256::from(0) {
        return Some(value);
    }
    if count == 255 {
        return (value == I256::from(-1)).then_some(I256::MIN);
    }
    value.checked_mul(I256::from(1) << count)
}

fn division_bounds(left: &FiniteHull, right: &IntegerDomain) -> Result<(I256, I256), Diagnostic> {
    let mut min = I256::MAX;
    let mut max = I256::MIN;
    for hull in [
        bounded_hull(right, I256::MIN, I256::from(-1)),
        bounded_hull(right, I256::from(1), I256::MAX),
    ]
    .into_iter()
    .flatten()
    {
        for divisor in [hull.min(), hull.max()] {
            for dividend in [left.min(), left.max()] {
                let quotient = dividend.checked_div(divisor).ok_or_else(overflow)?;
                min = min.min(quotient);
                max = max.max(quotient);
            }
        }
    }
    if min <= max {
        Ok((min, max))
    } else {
        Err(nonzero_divisor_required())
    }
}

fn remainder_bounds(left: &FiniteHull, right: &FiniteHull) -> (I256, I256) {
    let magnitude = |value: I256| {
        if value < I256::from(0) {
            -(value + I256::from(1))
        } else {
            value - I256::from(1)
        }
    };
    let maximum = magnitude(right.min())
        .max(magnitude(right.max()))
        .max(I256::from(0));
    let min = left.min().max(-maximum).min(I256::from(0));
    let max = left.max().min(maximum).max(I256::from(0));
    assert!(min <= max);
    (min, max)
}

fn overflow() -> Diagnostic {
    // type-integer-arithmetic-overflow
    Diagnostic::new(
        IntegerCode::ArithmeticOverflow,
        "integer arithmetic exceeds the supported 256-bit mathematical range",
    )
}

fn nonzero_divisor_required() -> Diagnostic {
    // type-integer-divisor-may-be-zero
    Diagnostic::new(
        IntegerCode::DivisorMayBeZero,
        "integer division and remainder require a divisor proven nonzero",
    )
}

fn valid_shift_count_required() -> Diagnostic {
    // type-integer-shift-count-unproven
    Diagnostic::new(
        IntegerCode::ShiftCountUnproven,
        "integer shift requires a count proven between 0 and 255",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_shift_preserves_the_signed_256_bit_boundary() {
        assert_eq!(checked_left_shift((-1).into(), 255), Some(I256::MIN));
        assert_eq!(checked_left_shift(0.into(), 255), Some(0.into()));
        assert_eq!(checked_left_shift(1.into(), 255), None);
        assert_eq!(checked_left_shift(I256::MIN, 0), Some(I256::MIN));
        assert_eq!(checked_left_shift(I256::MIN, 1), None);
    }

    #[test]
    fn minimum_divided_by_negative_one_exceeds_the_mathematical_limit() {
        let left = Ty::AnonymousInteger {
            validity: IntegerValidity::new(IntegerDomain::bounded(I256::MIN, I256::MIN).unwrap()),
        };
        let right = Ty::AnonymousInteger {
            validity: IntegerValidity::new(
                IntegerDomain::bounded((-1).into(), (-1).into()).unwrap(),
            ),
        };
        let values = [ConstValue::Int(I256::MIN), ConstValue::Int((-1).into())];
        let error = fold(IntrinsicOp::BitsSignedDivide, &values, [&left, &right])
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.code.slug().as_str(),
            "type-integer-arithmetic-overflow"
        );
        assert!(
            result_type(IntrinsicOp::BitsSignedDivide, [&left, &right], [None, None])
                .unwrap()
                .is_err()
        );
        assert_eq!(
            fold(IntrinsicOp::BitsSignedRemainder, &values, [&left, &right])
                .unwrap()
                .unwrap(),
            ConstValue::Int(0.into())
        );
        let result = result_type(
            IntrinsicOp::BitsSignedRemainder,
            [&left, &right],
            [None, None],
        )
        .unwrap()
        .unwrap();
        let hull = result
            .anonymous_integer_validity()
            .unwrap()
            .domain()
            .storage_hull()
            .unwrap();
        assert_eq!((hull.min(), hull.max()), (0.into(), 0.into()));
    }

    #[test]
    fn finite_domains_with_holes_enclose_every_mathematical_result() {
        let values = [-2_i16, -1, 0, 1, 2];
        let domain = |mask: u32| {
            let excluded = values
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    (mask & (1 << index) == 0).then_some(I256::from(*value))
                })
                .collect();
            Ty::AnonymousInteger {
                validity: IntegerValidity::new(
                    IntegerDomain::excluding((-2).into(), 2.into(), excluded).unwrap(),
                ),
            }
        };
        for left_mask in 1..32_u32 {
            for right_mask in 1..32_u32 {
                let left_type = domain(left_mask);
                let right_type = domain(right_mask);
                for op in [
                    IntrinsicOp::BitsAdd,
                    IntrinsicOp::BitsSubtract,
                    IntrinsicOp::BitsMultiply,
                    IntrinsicOp::BitsSignedDivide,
                    IntrinsicOp::BitsSignedRemainder,
                ] {
                    let result = result_type(op, [&left_type, &right_type], [None, None]).unwrap();
                    if matches!(
                        op,
                        IntrinsicOp::BitsSignedDivide | IntrinsicOp::BitsSignedRemainder
                    ) && right_mask & 4 != 0
                    {
                        assert_eq!(
                            result.unwrap_err().code.slug().as_str(),
                            "type-integer-divisor-may-be-zero"
                        );
                        continue;
                    }
                    let result = result.unwrap();
                    let result_domain = result.anonymous_integer_validity().unwrap().domain();
                    for (left_index, left) in values.iter().enumerate() {
                        for (right_index, right) in values.iter().enumerate() {
                            if left_mask & (1 << left_index) == 0
                                || right_mask & (1 << right_index) == 0
                            {
                                continue;
                            }
                            let expected = match op {
                                IntrinsicOp::BitsAdd => left + right,
                                IntrinsicOp::BitsSubtract => left - right,
                                IntrinsicOp::BitsMultiply => left * right,
                                IntrinsicOp::BitsSignedDivide => left / right,
                                IntrinsicOp::BitsSignedRemainder => left % right,
                                _ => unreachable!(),
                            };
                            assert!(
                                result_domain.contains(expected.into()),
                                "{left} {op:?} {right} = {expected}, inferred {result_domain:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}
