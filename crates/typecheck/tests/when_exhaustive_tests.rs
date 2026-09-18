use super::*;

#[test]
fn integer_coverage_handles_full_width_ranges_and_single_value_gaps() {
    let interval = |low: i128, high: i128| Pattern::InRange {
        bounds: ast::InRangeBounds::Literal(low.into(), high.into()),
        span: ast::SpanId::INVALID,
    };
    let lower = interval(i128::MIN, -1);
    let upper = interval(0, i128::MAX);
    assert!(integer_range_covered(
        i128::MIN,
        i128::MAX,
        &[&upper, &lower]
    ));
    let gap = interval(1, i128::MAX);
    assert!(!integer_range_covered(
        i128::MIN,
        i128::MAX,
        &[&lower, &gap]
    ));
    let zero = Pattern::Literal(ast::Literal::Int(0.into()), ast::SpanId::INVALID);
    assert!(integer_range_covered(
        i128::MIN,
        i128::MAX,
        &[&lower, &gap, &zero]
    ));
    assert!(!integer_range_covered(0, 0, &[]));
}
