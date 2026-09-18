use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;

mod support;
use support::intern;

#[test]
fn parses_explicit_anonymous_result_surface() {
    let ast = TokenCursor::parse_source("choose(value ref Int) True or False: True");
    let bind = &ast.defs[&intern("choose")];

    assert_eq!(
        bind.anonymous_result_alternatives
            .iter()
            .map(|alternative| alternative.value.label.as_str())
            .collect::<Vec<_>>(),
        ["True", "False"]
    );
    assert!(bind.return_tag.is_none());
}

#[test]
fn parses_proof_contracts_in_closed_proposition_context() {
    let ast = TokenCursor::parse_source(
        "less(left, right) True thus left < right or False thus left >= right: True",
    );
    let alternatives = &ast.defs[&intern("less")].anonymous_result_alternatives;

    assert!(matches!(
        alternatives.as_slice(),
        [
            ast::Spanned {
                value: ast::ResultAlternative {
                    label: true_label,
                    proposition: Some(ast::ProofProposition::Compare {
                        left: ast::ProofTerm::Name(left),
                        relation: ast::ProofRelation::Less,
                        right: ast::ProofTerm::Name(right),
                    }),
                },
                ..
            },
            ast::Spanned {
                value: ast::ResultAlternative {
                    label: false_label,
                    proposition: Some(ast::ProofProposition::Compare {
                        left: ast::ProofTerm::Name(false_left),
                        relation: ast::ProofRelation::GreaterOrEqual,
                        right: ast::ProofTerm::Name(false_right),
                    }),
                },
                ..
            }
        ] if true_label.as_str() == "True"
            && false_label.as_str() == "False"
            && left.as_str() == "left"
            && right.as_str() == "right"
            && false_left.as_str() == "left"
            && false_right.as_str() == "right"
    ));
}

#[test]
fn parses_named_result_rows_with_order_and_types() {
    let ast = TokenCursor::parse_source("add(lhs Int, rhs Int) sum Int, overflow Bit: lhs");
    let bind = &ast.defs[&intern("add")];

    assert_eq!(bind.named_results.len(), 2);
    assert_eq!(bind.named_results[0].name.as_str(), "sum");
    assert_eq!(bind.named_results[1].name.as_str(), "overflow");
    assert!(bind.named_results[0].name_span != ast::SpanId::INVALID);
    assert!(bind.named_results[1].name_span != ast::SpanId::INVALID);
}

#[test]
fn rejects_duplicate_named_results() {
    let ast = TokenCursor::parse_source("add() value Int, value Bit: 0");
    assert!(
        ast.parse_warnings
            .iter()
            .any(|warning| warning.code == "parse-duplicate-result-name".into())
    );
}

#[test]
fn parses_multiple_return_values_in_order() {
    let ast = TokenCursor::parse_source(
        "add(lhs Int, rhs Int) sum Int, overflow Bit:\n  return lhs, rhs\n",
    );
    let bind = &ast.defs[&intern("add")];
    let ast::BindValue::Body { ret, .. } = &bind.value else {
        panic!("expected block body");
    };

    assert_eq!(ret.values.len(), 2);
    assert!(matches!(ret.values[0].value, ast::Expr::FnCall(_)));
    assert!(matches!(ret.values[1].value, ast::Expr::FnCall(_)));
}

#[test]
fn preserves_multiple_values_in_indented_return() {
    let ast = TokenCursor::parse_source(
        "choose(flag Bit, lhs Int, rhs Int) first Int, second Int:\n\tif flag is True\n\t\treturn lhs, rhs\n\treturn lhs, rhs\n",
    );
    let bind = &ast.defs[&intern("choose")];
    let ast::BindValue::Body { exprs, .. } = &bind.value else {
        panic!("expected block body");
    };

    let ast::Expr::If(if_expr) = &exprs[0].value else {
        panic!("expected if expression");
    };
    assert_eq!(if_expr.body.len(), 2);
}

#[test]
fn rejects_parenthesized_positional_products() {
    let output = "value: (1, 2)".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "parse-positional-product-removed")
    );
    let bind = &output.ast.defs[&intern("value")];
    assert!(
        matches!(bind.value, ast::BindValue::Expr(ref expr) if matches!(expr.value, ast::Expr::Invalid(_)))
    );
}

#[test]
fn rejected_parenthesized_destructure_is_an_invalid_pattern() {
    let output = "main(value Int):\n\tif value is (left, right)\n\t\treturn 1\n\treturn 0"
        .parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.0 == "parse-positional-destructure-removed")
    );
    let bind = &output.ast.defs[&intern("main")];
    let ast::BindValue::Body { exprs, .. } = &bind.value else {
        panic!("expected block body");
    };
    let ast::Expr::If(if_expr) = &exprs[0].value else {
        panic!("expected if expression");
    };
    assert!(matches!(
        if_expr.condition,
        ast::Condition::Is { ref pattern, .. } if matches!(pattern.value, ast::Pattern::Invalid)
    ));
}

#[test]
fn has_methods_preserve_result_alternatives_and_propositions() {
    let ast = TokenCursor::parse_source(
        "Word is in 0 to 255\nWord has\n    compare(left Word, right Word) Lower thus left < right or Equal thus left = right or Higher thus left > right: Lower\n",
    );
    let bind = ast
        .defs
        .values()
        .find(|bind| !bind.anonymous_result_alternatives.is_empty())
        .unwrap();
    assert!(bind.return_tag.is_none());
    assert_eq!(
        bind.anonymous_result_alternatives
            .iter()
            .map(|alternative| alternative.value.label.as_str())
            .collect::<Vec<_>>(),
        ["Lower", "Equal", "Higher"]
    );
    assert!(
        bind.anonymous_result_alternatives
            .iter()
            .all(|alternative| alternative.value.proposition.is_some())
    );
}
