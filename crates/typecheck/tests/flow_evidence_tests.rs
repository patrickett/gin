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
    let evidence = typed.exprs.facts.semantic_facts.result_evidence[comparison.as_usize()]
        .as_ref()
        .expect("comparison evidence");

    for value in [exprs[0], moved, exprs[1], *returned] {
        assert_eq!(
            typed.exprs.facts.semantic_facts.result_evidence[value.as_usize()].as_ref(),
            Some(evidence)
        );
    }
    assert!(matches!(
        evidence.owner,
        ast::ResultFamilyOwner::Structural(ast::OperatorRole::Less)
    ));
}

#[test]
fn named_result_projection_survives_local_storage() {
    let source = "pair() first Int, second Int: return 1, 2\nmain: \n\tresult: pair()\n\tselected: result.second\n\treturn selected";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let main = typed.defs[&DefId(Intern::from_ref("main"))].clone();
    assert!(
        main.flaws
            .iter()
            .all(|flaw| flaw.code != "type-unknown-result-projection".into())
    );
    let BindBody::Body { exprs, .. } = main.body else {
        panic!("expected main body");
    };
    assert!(matches!(
        typed.exprs.kind[exprs[1].as_usize()],
        TypedExprKind::Bind { body, .. }
            if matches!(typed.exprs.kind[body.as_usize()], TypedExprKind::AggregateGet { index: 1, .. })
    ));
}

#[test]
fn callable_signature_preserves_named_result_order() {
    let source = "pair() first Int, second Bit: return 1, True\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let signature =
        typecheck::TypedCallableSignature::from(&typed.defs[&DefId(Intern::from_ref("pair"))]);
    assert_eq!(
        signature
            .results
            .iter()
            .map(|result| result.name.map(|name| name.to_string()))
            .collect::<Vec<_>>(),
        vec![Some("first".into()), Some("second".into())]
    );
    assert_eq!(signature.result_groups.len(), signature.results.len());
}

#[test]
fn one_named_result_normalizes_to_scalar_type() {
    let source = "identity(value Int) result Int: return value\nmain: return identity(1)";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let identity = &typed.defs[&DefId(Intern::from_ref("identity"))];
    assert!(matches!(identity.return_type, typecheck::ty::Ty::Opaque(_)));
    assert!(!matches!(
        identity.return_type,
        typecheck::ty::Ty::ResultRow(_)
    ));
    assert!(
        identity.flaws.is_empty(),
        "unexpected flaws: {:?}",
        identity.flaws
    );
}

#[test]
fn unnamed_multiple_returns_are_rejected_instead_of_becoming_value_rows() {
    let typed = transform(
        &TokenCursor::parse_source("pair: return 1, 2"),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        typed
            .all_flaws()
            .into_iter()
            .any(|(_, flaw)| flaw.code == "type-unnamed-multiple-results".into())
    );
}

#[test]
fn named_results_preserve_individual_reference_groups() {
    let source = "pair(lhs ref{left} Int, rhs ref{right} Int) left ref{left} Int, right ref{right} Int: return lhs, rhs\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let bind = &typed.defs[&DefId(Intern::from_ref("pair"))];
    assert_eq!(bind.result_groups.len(), 2);
    assert!(bind.result_groups[0].is_some());
    assert!(bind.result_groups[1].is_some());
    assert_ne!(bind.result_groups[0], bind.result_groups[1]);
}

#[test]
fn named_result_rows_forward_through_callable_calls() {
    let source = "pair() first Int, second Int: return 1, 2\nforward() first Int, second Int: return pair()\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let forward = &typed.defs[&DefId(Intern::from_ref("forward"))];
    assert!(
        forward.flaws.is_empty(),
        "unexpected flaws: {:?}",
        forward.flaws
    );
    let pair = &typed.defs[&DefId(Intern::from_ref("pair"))];
    let BindBody::Body {
        ret: Some(pair_ret),
        ..
    } = pair.body
    else {
        panic!("expected pair return body");
    };
    assert!(matches!(
        typed.exprs.kind[pair_ret.as_usize()],
        TypedExprKind::ResultRow(_)
    ));
    let typecheck::ty::Ty::ResultRow(fields) = &forward.return_type else {
        panic!("expected named result row, got {:?}", forward.return_type);
    };
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn incompatible_named_result_forwarding_is_rejected() {
    let source = "pair() first Int, second Int: return 1, 2\nforward() first Bool, second Int: return pair()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let forward = &typed.defs[&DefId(Intern::from_ref("forward"))];
    let mismatch = forward
        .flaws
        .iter()
        .find(|flaw| flaw.code == "type-return-mismatch".into())
        .expect("forwarding mismatch");
    let start = source.rfind("pair()").unwrap();
    assert_eq!(
        mismatch.span.start() as usize..mismatch.span.end() as usize,
        start..start + "pair()".len()
    );
}

#[test]
fn short_named_result_return_reports_shape_mismatch() {
    let source = "Byte is in 0 to 255\npair() first Byte, second Byte: return 1\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let pair = &typed.defs[&DefId(Intern::from_ref("pair"))];
    assert!(
        pair.flaws
            .iter()
            .any(|flaw| flaw.code == "type-return-mismatch".into()),
        "expected row-shape diagnostic, got {:?}",
        pair.flaws
    );
}

#[test]
fn missing_named_result_reports_return_arity() {
    let source = "Byte is in 0 to 255\npair() first Byte, second Byte: return 1\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let pair = &typed.defs[&DefId(Intern::from_ref("pair"))];
    assert!(
        pair.flaws
            .iter()
            .any(|flaw| flaw.code == "type-return-arity".into())
    );
}

#[test]
fn excess_named_results_report_return_arity() {
    let source = "Byte is in 0 to 255\npair() first Byte, second Byte: return 1, 2, 3\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let pair = &typed.defs[&DefId(Intern::from_ref("pair"))];
    assert!(
        pair.flaws
            .iter()
            .any(|flaw| flaw.code == "type-return-arity".into())
    );
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
    let first = typed.exprs.facts.semantic_facts.result_evidence[first_comparison.as_usize()]
        .as_ref()
        .expect("first evidence");
    let second = typed.exprs.facts.semantic_facts.result_evidence[second_comparison.as_usize()]
        .as_ref()
        .expect("second evidence");

    assert_eq!(
        typed.exprs.facts.semantic_facts.result_evidence[old_read.as_usize()].as_ref(),
        Some(first)
    );
    assert_eq!(
        typed.exprs.facts.semantic_facts.result_evidence[new_read.as_usize()].as_ref(),
        Some(second)
    );
    assert_ne!(first, second);
}

#[test]
fn positional_projection_of_named_result_reports_diagnostic() {
    let source =
        "pair() first Int, second Int: return 1, 2\nmain:\n\tresult: pair()\n\treturn result.0";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let main = &typed.defs[&DefId(Intern::from_ref("main"))];
    assert!(
        main.flaws
            .iter()
            .any(|flaw| flaw.code == "type-positional-result-projection".into())
    );
}

#[test]
fn compatible_named_result_rows_join_through_when() {
    let source = "Bool is True or False\nfirst() sum Int, overflow Int: return 1, 0\nsecond() sum Int, overflow Int: return 2, 0\nmain(flag Bool): return when flag is True then first() else second()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let main = &typed.defs[&DefId(Intern::from_ref("main"))];
    assert!(main.flaws.is_empty(), "unexpected flaws: {:?}", main.flaws);
    let typecheck::ty::Ty::ResultRow(elements) = &main.return_type else {
        panic!("expected named result row, got {:?}", main.return_type);
    };
    assert_eq!(
        elements
            .iter()
            .map(|element| element.name.as_str())
            .collect::<Vec<_>>(),
        ["sum", "overflow"]
    );
}

#[test]
fn incompatible_named_result_rows_are_rejected_by_when() {
    let source = "Bool is True or False\nfirst() sum Int, overflow Int: return 1, 0\nsecond() value Int, carry Int: return 2, 0\nmain(flag Bool): return when flag is True then first() else second()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        typed
            .exprs
            .facts
            .semantic_facts
            .flaws
            .iter()
            .flatten()
            .any(|flaw| flaw.code == "type-incompatible-result-names".into()),
        "incompatible result names were accepted"
    );
}

#[test]
fn named_result_type_mismatches_are_reported_at_the_return_values() {
    let source = "Byte is in 0 to 255\nBool is True or False\nbyte() Byte: return 1\ntruth() Bool: return True\npair() first Bool, second Byte: return byte(), truth()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("pair"))].flaws;
    assert_eq!(
        flaws
            .iter()
            .filter(|flaw| flaw.code == "type-return-mismatch".into())
            .count(),
        2,
        "{flaws:?}"
    );
    let spans = flaws
        .iter()
        .filter(|flaw| flaw.code == "type-return-mismatch".into())
        .map(|flaw| flaw.span.start() as usize..flaw.span.end() as usize)
        .collect::<Vec<_>>();
    let byte = source.rfind("byte()").unwrap();
    let truth = source.rfind("truth()").unwrap();
    assert_eq!(
        spans,
        [byte..byte + "byte()".len(), truth..truth + "truth()".len()]
    );
}

#[test]
fn conditional_return_type_mismatches_are_reported_at_branch_values() {
    let source = "Bool is True or False\nByte is in 0 to 255\nbyte() Byte: return 1\ntruth() Bool: return True\nchoose(flag Bool) first Bool, second Byte:\n    if flag is True\n        return byte(), truth()\n    return\n    return truth(), byte()";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = &typed.defs[&DefId(Intern::from_ref("choose"))].flaws;
    let mismatches = flaws
        .iter()
        .filter(|flaw| flaw.code == "type-return-mismatch".into())
        .collect::<Vec<_>>();
    assert_eq!(mismatches.len(), 2, "{flaws:?}");
    let spans = mismatches
        .iter()
        .map(|flaw| flaw.span.start() as usize..flaw.span.end() as usize)
        .collect::<Vec<_>>();
    let byte = source.find("return byte(), truth()").unwrap() + "return ".len();
    let truth = byte + "byte(), ".len();
    assert_eq!(
        spans,
        [byte..byte + "byte()".len(), truth..truth + "truth()".len()]
    );
}

#[test]
fn conditional_return_arity_failures_are_independent() {
    for (values, expected) in [("byte()", 1usize), ("byte(), byte(), byte()", 3usize)] {
        let source = format!(
            "Bool is True or False\nByte is in 0 to 255\nbyte() Byte: return 1\nchoose(flag Bool) first Byte, second Byte:\n    if flag is True\n        return {values}\n    return\n    return 1, 2"
        );
        let typed = transform(
            &TokenCursor::parse_source(&source),
            FileId(0),
            &TransformCtx::new(),
        );
        let flaws = &typed.defs[&DefId(Intern::from_ref("choose"))].flaws;
        let arity = flaws
            .iter()
            .filter(|flaw| flaw.code == "type-return-arity".into())
            .collect::<Vec<_>>();
        assert_eq!(
            arity.len(),
            1,
            "expected {expected} branch values: {flaws:?}"
        );
        let start = source.find("return byte()").unwrap() + "return ".len();
        assert_eq!(
            arity[0].span.start() as usize..arity[0].span.end() as usize,
            start..start + "byte()".len()
        );
    }
}
