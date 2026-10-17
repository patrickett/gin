mod support;

use ast::{ConstValue, NormalExpr};
use internment::Intern;
use typecheck::{DefId, TypedExprKind};

fn evaluate_repeat(count: i128) -> Result<ConstValue, diagnostic::Diagnostic> {
    let mut typed = support::check("values: [1; 2]\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let expression = support::body(&typed, "values");
    let TypedExprKind::FixedArrayAlloc { init, .. } = &typed.exprs.kind[expression.index()] else {
        panic!("expected an inferred repeat");
    };
    let seed = *init;
    typed.exprs.kind[expression.index()] = TypedExprKind::FixedArrayAlloc {
        init: seed,
        size: NormalExpr::from(count),
    };
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    )
    .call(DefId::named(Intern::from_ref("values")), &[])
}

#[test]
fn repeat_allocations_reject_invalid_counts() {
    for count in [-1, usize::MAX as i128] {
        let diagnostic = evaluate_repeat(count).expect_err("invalid allocation must fail");
        assert_eq!(
            diagnostic.code.slug().as_str(),
            "compile-time-invalid-allocation"
        );
    }
}

#[test]
fn repeat_allocations_preserve_empty_and_nonempty_values() {
    for count in [0, 3] {
        assert_eq!(
            evaluate_repeat(count).unwrap(),
            ConstValue::FixedArray(vec![ConstValue::ONE; count as usize].into())
        );
    }
}
