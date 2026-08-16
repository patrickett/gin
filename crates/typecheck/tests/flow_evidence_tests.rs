use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

#[test]
fn structural_result_evidence_survives_local_bind_move_and_return() {
    let source = "main:\n\tcomparison: 1 < 2\n\tmoved: comparison\n\treturn moved";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Body {
        exprs,
        ret: Some(returned),
    } = &typed.defs[&DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected body");
    };
    let bind_body = |expr: typecheck::ExprId| match typed.exprs.kind[expr.as_usize()] {
        TypedExprKind::Bind { body, .. } => body,
        ref kind => panic!("expected bind, got {kind:?}"),
    };
    let comparison = bind_body(exprs[0]);
    let moved = bind_body(exprs[1]);
    let evidence = typed.exprs.result_evidence[comparison.as_usize()]
        .as_ref()
        .expect("comparison evidence");

    for value in [exprs[0], moved, exprs[1], *returned] {
        assert_eq!(
            typed.exprs.result_evidence[value.as_usize()].as_ref(),
            Some(evidence)
        );
    }
    assert!(matches!(
        evidence.owner,
        ast::ResultFamilyOwner::Structural(ast::OperatorRole::Less)
    ));
}

#[test]
fn structural_result_evidence_survives_tuple_storage_and_projection() {
    let source = "main:\n\tcomparison: 1 < 2\n\tstored: (comparison, 0)\n\tprojected: stored.0\n\treturn projected";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Body {
        exprs,
        ret: Some(returned),
    } = &typed.defs[&DefId(Intern::from_ref("main"))].body
    else {
        panic!("expected body");
    };
    let bind_body = |expr: typecheck::ExprId| match typed.exprs.kind[expr.as_usize()] {
        TypedExprKind::Bind { body, .. } => body,
        ref kind => panic!("expected bind, got {kind:?}"),
    };
    let comparison = bind_body(exprs[0]);
    let stored = bind_body(exprs[1]);
    let projected = bind_body(exprs[2]);
    let evidence = typed.exprs.result_evidence[comparison.as_usize()]
        .as_ref()
        .expect("comparison evidence");

    assert_eq!(
        typed.exprs.projected_result_evidence[stored.as_usize()].get(&vec![0]),
        Some(evidence)
    );
    for value in [projected, exprs[2], *returned] {
        assert_eq!(
            typed.exprs.result_evidence[value.as_usize()].as_ref(),
            Some(evidence)
        );
    }
}

#[test]
fn rebinding_keeps_old_and_new_result_evidence_separate() {
    let source = "main:\n\tcomparison: 1 < 2\n\told: comparison\n\tcomparison:: 3 < 4\n\tnew: comparison\n\treturn new";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Body { exprs, .. } = &typed.defs[&DefId(Intern::from_ref("main"))].body else {
        panic!("expected body");
    };
    let bind_body = |expr: typecheck::ExprId| match typed.exprs.kind[expr.as_usize()] {
        TypedExprKind::Bind { body, .. } => body,
        ref kind => panic!("expected bind, got {kind:?}"),
    };
    let first_comparison = bind_body(exprs[0]);
    let old_read = bind_body(exprs[1]);
    let TypedExprKind::Reassign {
        value: second_comparison,
        ..
    } = typed.exprs.kind[exprs[2].as_usize()]
    else {
        panic!("expected reassignment");
    };
    let new_read = bind_body(exprs[3]);
    let first = typed.exprs.result_evidence[first_comparison.as_usize()]
        .as_ref()
        .expect("first evidence");
    let second = typed.exprs.result_evidence[second_comparison.as_usize()]
        .as_ref()
        .expect("second evidence");

    assert_eq!(
        typed.exprs.result_evidence[old_read.as_usize()].as_ref(),
        Some(first)
    );
    assert_eq!(
        typed.exprs.result_evidence[new_read.as_usize()].as_ref(),
        Some(second)
    );
    assert_ne!(first, second);
}
