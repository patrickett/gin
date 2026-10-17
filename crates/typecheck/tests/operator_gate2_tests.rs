mod support;

use internment::Intern;
use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;
use support::flaw_codes_of;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform as do_transform};
use typecheck::{BindBody, DefId};

fn result_ty(source: &str, name: &str) -> ast::Ty {
    let ast = TokenCursor::parse_source(source);
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let body = match &typed.defs[&DefId::named(Intern::new(name.to_string()))].body {
        BindBody::Expr(expression) => *expression,
        _ => panic!("expected expression body"),
    };
    typed.exprs.ty[body.as_usize()].clone()
}

#[test]
fn nominal_operator_missing_reports_nominal_operator_unavailable() {
    let source = format!(
        "\n{}Small is in 0 to 31\nadd_small_word(lhs own Small, rhs own Word) Word: return rhs\n#infix('+')\n#inline\nright_only(lhs Small, rhs Word) Word: return add_small_word(lhs, rhs)\nleft: Word: 1\nright: Word: 2\nbad_op: left + right\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let codes = flaw_codes_of(&source);
    assert!(codes.contains(&"type-nominal-operator-unavailable".to_string()));
}

#[test]
fn infix_declaration_requires_exactly_two_value_parameters() {
    let source = "\nWord is in 0 to 10\nWord has\n    finish(self):\n        return\n#infix('+')\nunary(value Word) Word: return value\n";
    let codes = flaw_codes_of(source);
    assert!(
        codes.contains(&"type-invalid-infix-declaration".to_string()),
        "expected infix arity diagnostic, got {codes:?}"
    );
}

#[test]
fn named_comparison_preserves_declared_result_and_owned_conventions() {
    let source = format!(
        "\n{}Decision is Lower or NotLower\nless(lhs own Word, rhs own Word) Decision: return Decision.Lower\nleft Word: 1\nright Word: 2\nresult: less(left, right)\n",
        test_fixtures::preludes::WORD_PRELUDE
    );

    assert!(matches!(
        result_ty(&source, "result"),
        ast::Ty::Named { name, .. } if name.as_str() == "Decision"
    ));
    let codes = flaw_codes_of(&source);
    assert_eq!(
        codes
            .iter()
            .filter(|code| *code == "type-owned-value-not-consumed")
            .count(),
        2,
        "{codes:?}"
    );
    assert_eq!(
        codes
            .iter()
            .filter(|code| *code == "type-owned-param-not-consumed")
            .count(),
        2,
        "{codes:?}"
    );
}

#[test]
fn named_comparison_ref_conventions_observe_without_consuming() {
    let source = format!(
        "\n{}Decision is Lower or NotLower\nless(lhs ref Word, rhs ref Word) Decision: return Decision.Lower\nleft Word: 1\nright Word: 2\nresult: less(left, right)\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let codes = flaw_codes_of(&source);

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
fn numeric_comparison_preserves_the_literal_rhs_domain() {
    let source =
        "Word is in 0 to 255\nWord has finish(self): return\nleft Word: 1\nresult: left < 256\n";
    let ast = TokenCursor::parse_source(source);
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let BindBody::Expr(result) = typed.defs[&DefId::named(Intern::from_ref("result"))].body else {
        panic!("comparison body");
    };
    let typecheck::TypedExprKind::IntrinsicCall { args, .. } = &typed.exprs.kind[result.index()]
    else {
        panic!("fixed numeric comparison");
    };
    assert!(matches!(
        typed.exprs.ty[args[1].index()],
        ast::Ty::AnonymousInteger { .. }
    ));
    assert_eq!(
        typed.exprs.const_value_of(args[1]).cloned().flatten(),
        Some(ast::ConstValue::Int(256.into()))
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn anonymous_integer_less_is_a_less_proposition() {
    assert!(matches!(
        result_ty("lt: 1 < 2", "lt"),
        ast::Ty::Proposition(predicate) if matches!(*predicate, ast::ProofProposition::Compare { relation: ast::ProofRelation::Less, .. })
    ));
}

#[test]
fn anonymous_integer_less_equal_is_a_less_equal_proposition() {
    assert!(matches!(
        result_ty("le: 1 <= 2", "le"),
        ast::Ty::Proposition(predicate) if matches!(*predicate, ast::ProofProposition::Compare { relation: ast::ProofRelation::LessOrEqual, .. })
    ));
}

#[test]
fn anonymous_integer_greater_is_a_greater_proposition() {
    assert!(matches!(
        result_ty("gt: 2 > 1", "gt"),
        ast::Ty::Proposition(predicate) if matches!(*predicate, ast::ProofProposition::Compare { relation: ast::ProofRelation::Greater, .. })
    ));
}

#[test]
fn anonymous_integer_greater_equal_is_a_greater_equal_proposition() {
    assert!(matches!(
        result_ty("ge: 2 >= 1", "ge"),
        ast::Ty::Proposition(predicate) if matches!(*predicate, ast::ProofProposition::Compare { relation: ast::ProofRelation::GreaterOrEqual, .. })
    ));
}

#[test]
fn anonymous_integer_equal_is_an_equal_proposition() {
    assert!(matches!(
        result_ty("eq: 1 = 2", "eq"),
        ast::Ty::Proposition(predicate) if matches!(*predicate, ast::ProofProposition::Compare { relation: ast::ProofRelation::Equal, .. })
    ));
}

#[test]
fn propositions_are_not_numeric_operands() {
    let codes = flaw_codes_of("less: 1 < 2\nequal: 1 = 2\nbad: less = equal\n");
    assert!(
        codes.contains(&"type-numeric-predicate-operands".to_string()),
        "{codes:?}"
    );
}

#[test]
fn less_records_proposition_evidence() {
    let ast = TokenCursor::parse_source("less: 1 < 2");
    let typed = do_transform(&ast, FileId(0), &TransformCtx::new());
    let body = match &typed.defs[&DefId::named(Intern::from_ref("less"))].body {
        BindBody::Expr(expression) => *expression,
        _ => panic!("expected expression body"),
    };
    let evidence = typed.exprs.facts.semantic_facts.result_evidence[body.as_usize()]
        .as_ref()
        .expect("structural comparison evidence");

    assert!(matches!(
        evidence.as_ref(),
        typecheck::ValueEvidence::Proposition(typecheck::EvidenceProposition::IntegerComparison {
            comparison: typecheck::MathematicalComparison::Less,
            ..
        })
    ));
}

#[test]
fn numeric_domains_compare_in_both_directions() {
    let source = "Int is in 0 to 255\nInt has finish(self): return\nvalue Int: 10\nfrom_named: 1 < value\nto_named: value < (2 + 0)\n";
    let codes = flaw_codes_of(source);
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn is_in_range_pattern_bypasses_nominal_comparison_dispatch() {
    let source = format!(
        "\n{}Decision is Inside or Outside\nequal(lhs ref Word, rhs ref Word) Decision: return Decision.Inside\nclassify(value Word) Word: return when value is in 0 to 127 then 1\n                              else 2\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let typed = do_transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    let BindBody::Expr(body) = typed.defs[&DefId::named(Intern::from_ref("classify"))].body else {
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

#[test]
fn shift_operator_can_return_an_explicit_error_union() {
    let source = format!(
        "\n{}ShiftResult is Value(value Word) or InvalidShift\n#infix('<<')\nshift(lhs Word, rhs Word) ShiftResult: return InvalidShift\nleft Word: 1\nright Word: 8\nresult: left << right\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let codes = flaw_codes_of(&source);
    assert!(
        !codes.contains(&"type-invalid-operator-result".to_string()),
        "{codes:?}"
    );
    assert!(
        matches!(result_ty(&source, "result"), ast::Ty::Named { name, .. } if name.as_str() == "ShiftResult")
    );
}

#[test]
fn self_in_numeric_methods_resolves_parameters_results_and_casts() {
    let source = format!(
        "\n{}    #infix('+')\n    add(lhs Self, rhs Self) Self:\n        left is in 0 to 255: lhs\n        right is in 0 to 255: rhs\n        return ((left + right) & 255) as Self\n    copy(value ref Self) Self:\n        bits is in 0 to 255: value\n        return bits as Self\n    replace(previous Self, next Self) Self:\n        discarded is in 0 to 255: previous\n        return next\n\nuse_word(lhs Word, rhs Word) Word: return lhs + rhs\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let codes = flaw_codes_of(&source);
    assert!(codes.is_empty(), "{codes:?}");
    assert!(
        matches!(result_ty(&source, "use_word"), ast::Ty::Named { name, .. } if name.as_str() == "Word")
    );
}
