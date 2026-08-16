use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform as do_transform};
use typecheck::{BindBody, DefId};

fn flaws(source: &str) -> Vec<String> {
    let ast = TokenCursor::parse_source(source);
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}

fn result_ty(source: &str, name: &str) -> ast::Ty {
    let ast = TokenCursor::parse_source(source);
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let body = match &typed.defs[&DefId(Intern::new(name.to_string()))].body {
        BindBody::Expr(expression) => *expression,
        _ => panic!("expected expression body"),
    };
    typed.exprs.ty[body.as_usize()].clone()
}

#[test]
fn nominal_operator_missing_reports_nominal_operator_unavailable() {
    let source = "
Word is in 0...255
Small is in 0...31
#intrinsic(AddSmallWord)
add_small_word(eat lhs Small, eat rhs Word) Word extern
#operator(Add)
#inline
right_only(lhs Small, rhs Word) Word: add_small_word(eat lhs, eat rhs)
left: Word: 1
right: Word: 2
bad_op: left + right
";
    let codes = flaws(source);
    assert!(codes.contains(&"type-nominal-operator-unavailable".to_string()));
}

#[test]
fn nominal_comparison_dispatch_preserves_declared_result_and_owned_conventions() {
    let source = "
Word is in 0...255
Decision is Lower or NotLower
#operator(Less)
less(eat lhs Word, eat rhs Word) Decision: Decision.Lower
left Word: 1
right Word: 2
result: left < right
";

    assert!(matches!(
        result_ty(source, "result"),
        ast::Ty::Named { name, .. } if name.as_str() == "Decision"
    ));
    let codes = flaws(source);
    assert_eq!(
        codes
            .iter()
            .filter(|code| *code == "type-consume-required")
            .count(),
        2,
        "{codes:?}"
    );
}

#[test]
fn nominal_comparison_ref_conventions_observe_without_consuming() {
    let source = "
Word is in 0...255
Decision is Lower or NotLower
#operator(Less)
less(ref lhs Word, ref rhs Word) Decision: Decision.Lower
left Word: 1
right Word: 2
result: left < right
";
    let codes = flaws(source);

    assert!(
        !codes.iter().any(|code| code == "type-consume-required"),
        "{codes:?}"
    );
    assert!(
        !codes.iter().any(|code| code == "type-use-after-move"),
        "{codes:?}"
    );
}

#[test]
fn nominal_left_operand_contextualizes_unresolved_literal_rhs() {
    let source = "
Word is in 0...255
Decision is Lower or NotLower
#operator(Less)
less(lhs Word, rhs Word) Decision: Decision.Lower
left Word: 1
result: left < 10
";
    let ast = TokenCursor::parse_source(source);
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let BindBody::Expr(result) = typed.defs[&DefId(Intern::from_ref("result"))].body else {
        panic!("expected comparison body");
    };
    let typecheck::TypedExprKind::FnCall {
        target,
        args: Some(args),
        ..
    } = &typed.exprs.kind[result.as_usize()]
    else {
        panic!(
            "expected nominal operator call, got {:?}",
            typed.exprs.kind[result.as_usize()]
        );
    };

    assert_eq!(target.0.as_str(), "less");
    assert_eq!(
        typed.exprs.ty[args[1].as_usize()],
        typed.exprs.ty[args[0].as_usize()]
    );
    assert!(matches!(
        &typed.exprs.ty[args[1].as_usize()],
        ast::Ty::Named { name, .. } if name.as_str() == "Word"
    ));
    assert!(
        typed
            .all_flaws()
            .iter()
            .all(|(_, flaw)| { flaw.code.slug() != "type-mixed-anonymous-nominal-comparison" })
    );
}

#[test]
fn anonymous_integer_less_uses_less_result_family() {
    assert!(matches!(
        result_ty("lt: 1 < 2", "lt"),
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Less),
            ..
        }
    ));
}

#[test]
fn anonymous_integer_less_equal_uses_less_equal_result_family() {
    assert!(matches!(
        result_ty("le: 1 <= 2", "le"),
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::LessOrEqual),
            ..
        }
    ));
}

#[test]
fn anonymous_integer_greater_uses_greater_result_family() {
    assert!(matches!(
        result_ty("gt: 2 > 1", "gt"),
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Greater),
            ..
        }
    ));
}

#[test]
fn anonymous_integer_greater_equal_uses_greater_equal_result_family() {
    assert!(matches!(
        result_ty("ge: 2 >= 1", "ge"),
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::GreaterOrEqual),
            ..
        }
    ));
}

#[test]
fn anonymous_integer_equal_uses_equal_result_family() {
    assert!(matches!(
        result_ty("eq: 1 = 2", "eq"),
        ast::Ty::ResultFamily {
            owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Equal),
            ..
        }
    ));
}

#[test]
fn structural_operator_families_with_same_labels_are_incompatible() {
    let codes = flaws("less: 1 < 2\nequal: 1 = 2\nbad: less = equal\n");
    assert!(
        codes.contains(&"type-result-family-incompatible".to_string()),
        "{codes:?}"
    );
}

#[test]
fn structural_less_records_true_and_false_evidence_on_the_value() {
    let ast = TokenCursor::parse_source("less: 1 < 2");
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let body = match &typed.defs[&DefId(Intern::from_ref("less"))].body {
        BindBody::Expr(expression) => *expression,
        _ => panic!("expected expression body"),
    };
    let evidence = typed.exprs.result_evidence[body.as_usize()]
        .as_ref()
        .expect("structural comparison evidence");

    assert!(matches!(
        evidence.owner,
        ast::ResultFamilyOwner::Structural(ast::OperatorRole::Less)
    ));
    assert!(matches!(
        evidence.alternatives.as_slice(),
        [
            typecheck::AlternativeEvidence {
                label: false_label,
                proposition: typecheck::EvidenceProposition::IntegerComparison {
                    comparison: typecheck::MathematicalComparison::GreaterOrEqual,
                    ..
                }
            },
            typecheck::AlternativeEvidence {
                label: true_label,
                proposition: typecheck::EvidenceProposition::IntegerComparison {
                    comparison: typecheck::MathematicalComparison::Less,
                    ..
                }
            }
        ] if false_label.as_str() == "False" && true_label.as_str() == "True"
    ));
}

#[test]
fn mixed_anonymous_nominal_comparison_is_rejected_in_both_directions() {
    let source = "
Int is in 0...255
value Int: 10
from_named: 1 < value
to_named: value < (2 + 0)
";
    let codes = flaws(source);
    assert_eq!(
        codes
            .iter()
            .filter(|code| *code == "type-mixed-anonymous-nominal-comparison")
            .count(),
        2,
        "{codes:?}"
    );
}

#[test]
fn anonymous_left_nominal_right_comparison_is_rejected() {
    let codes = flaws("Int is in 0...255\nvalue Int: 10\nbad: 1 < value\n");
    assert!(
        codes
            .iter()
            .any(|code| code == "type-mixed-anonymous-nominal-comparison"),
        "{codes:?}"
    );
}

#[test]
fn nominal_left_anonymous_right_comparison_is_rejected() {
    let codes = flaws("Int is in 0...255\nvalue Int: 10\nbad: value < (1 + 1)\n");
    assert!(
        codes
            .iter()
            .any(|code| code == "type-mixed-anonymous-nominal-comparison"),
        "{codes:?}"
    );
}

#[test]
fn is_in_range_pattern_bypasses_nominal_comparison_dispatch() {
    let source = "
Word is in 0...255
Decision is Inside or Outside
#operator(Equal)
equal(ref lhs Word, ref rhs Word) Decision: Decision.Inside
classify(value Word) Word: when value is in 0...127 then 1
                              else 2
";
    let typed = do_transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(body) = typed.defs[&DefId(Intern::from_ref("classify"))].body else {
        panic!("expected expression body");
    };
    let typecheck::TypedExprKind::When(when_expr) = &typed.exprs.kind[body.as_usize()] else {
        panic!("expected pattern when");
    };
    let subject = when_expr.subject.expect("pattern when subject");

    assert!(matches!(
        &typed.exprs.ty[subject.as_usize()],
        ast::Ty::Named { name, .. } if name.as_str() == "Word"
    ));
    assert!(matches!(
        &when_expr.arms[0],
        typecheck::TypedWhenArm::Is { pattern, .. }
            if matches!(pattern.value, ast::Pattern::InRange { .. })
    ));
    assert!(!typed.exprs.kind.iter().any(|kind| matches!(
        kind,
        typecheck::TypedExprKind::FnCall { target, .. }
            if target.0.as_str() == "equal"
    )));
}
