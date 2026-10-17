use diagnostic::DiagnosticCode;
use diagnostic::code::{ParseCode, SyntaxCode};
use parser::cursor::TokenCursor;
use parser::query::SourceParseExt;

mod support;
use support::intern;

#[test]
fn parses_direct_generic_union_return_types_in_functions_and_methods() {
    for source in [
        "choose(x) Block(x) or Error or Other(x): return Empty",
        "Store has\n    choose(self mut, x) Block(x) or Error or Other(x)\n",
        "choose(x) Error or Block(x): return Empty",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{source}: {:?}",
            parsed.symptoms
        );
        let surface = if let Some(bind) = parsed.ast.defs.get(&intern("choose")) {
            bind.return_tag.as_deref().unwrap()
        } else {
            let ast::DeclareValue::Has(members) = &parsed.ast.tags[&intern("Store")].value else {
                panic!("expected store members");
            };
            let ast::HasMember::Function(method) = &members[0] else {
                panic!("expected method");
            };
            method.return_ty.as_deref().unwrap()
        };
        assert!(
            matches!(&surface.value, ast::Expr::Binary(binary) if binary.op == ast::BinOp::LogicalOr)
        );
    }
}

#[test]
fn rejects_missing_direct_union_member() {
    let parsed = "choose(x) Block(x) or: return Empty".parse_source_full();
    assert!(
        parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-expected-result-alternative")
    );
}

#[test]
fn parses_explicit_anonymous_result_surface() {
    let ast = TokenCursor::parse_source("choose(value ref Int) True or False: return True");
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
        "less(left, right) True thus left < right or False thus left >= right: return True",
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
    let ast = TokenCursor::parse_source("add(lhs Int, rhs Int) sum Int, overflow Bit: return lhs");
    let bind = &ast.defs[&intern("add")];

    assert_eq!(bind.named_results.len(), 2);
    assert_eq!(bind.named_results[0].name.as_ref().unwrap().as_str(), "sum");
    assert_eq!(
        bind.named_results[1].name.as_ref().unwrap().as_str(),
        "overflow"
    );
    assert!(bind.named_results[0].name_span != ast::SpanId::INVALID);
    assert!(bind.named_results[1].name_span != ast::SpanId::INVALID);
}

#[test]
fn rejects_duplicate_named_results() {
    let ast = TokenCursor::parse_source("add() value Int, value Bit: return 0");
    assert!(ast.parse_warnings.iter().any(|warning| warning.code
        == DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::DuplicateResultName))));
}

#[test]
fn parses_multiple_return_values_in_order() {
    let ast = TokenCursor::parse_source(
        "add(lhs Int, rhs Int) sum Int, overflow Bit:\n    return lhs, rhs\n",
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
        "choose(flag Bit, lhs Int, rhs Int) first Int, second Int:\n\twhen flag is True then\n\t\treturn lhs, rhs\n\treturn lhs, rhs\n",
    );
    let bind = &ast.defs[&intern("choose")];
    let ast::BindValue::Body { exprs, .. } = &bind.value else {
        panic!("expected block body");
    };

    let ast::Expr::When(branch) = &exprs[0].value else {
        panic!("expected when expression");
    };
    let ast::WhenArm::Is { pattern, body, .. } = &branch.arms[0] else {
        panic!()
    };
    assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
    let ast::Expr::Block(items) = &body.value else {
        panic!()
    };
    let ast::Expr::Return(ret) = &items[0].value else {
        panic!()
    };
    assert_eq!(ret.values.len(), 2);
    assert!(matches!(pattern.value, ast::Pattern::Nominal(..)));
}

#[test]
fn rejects_parenthesized_positional_products() {
    let output = "value: (1, 2)".parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-parenthesized-sequence")
    );
    let bind = &output.ast.defs[&intern("value")];
    assert!(
        matches!(bind.value, ast::BindValue::Expr(ref expr) if matches!(expr.value, ast::Expr::Invalid(_)))
    );
}

#[test]
fn rejected_parenthesized_destructure_is_an_invalid_pattern() {
    let output =
        "main(value Int):\n\twhen value is (left, right) then\n\t\t\treturn 1\n\t\treturn 0"
            .parse_source_full();
    assert!(
        output
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-positional-destructure-removed")
    );
    let bind = &output.ast.defs[&intern("main")];
    let ast::BindValue::Body { exprs, .. } = &bind.value else {
        panic!("expected block body");
    };
    let ast::Expr::When(branch) = &exprs[0].value else {
        panic!("expected when expression");
    };
    let ast::WhenArm::Is { pattern, .. } = &branch.arms[0] else {
        panic!()
    };
    assert!(matches!(pattern.value, ast::Pattern::Invalid));
}

#[test]
fn has_methods_preserve_result_alternatives_and_propositions() {
    let ast = TokenCursor::parse_source(
        "Word is in 0 to 255\nWord has\n    compare(left Word, right Word) Lower thus left < right or Equal thus left = right or Higher thus left > right: return Lower\n",
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

#[test]
fn parses_named_payload_contracts_in_free_functions_and_methods() {
    for source in [
        "reserve(c Count) storage Allocation(x) thus storage.initialized_count = 0 and storage.capacity_count >= c or AllocError: return OutOfMemory\n",
        "Allocator has\n    reserve(self mut, c Count(x)) storage Allocation(x) thus storage.initialized_count = 0 and storage.capacity_count >= c or AllocError\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{:?}\n{source}",
            parsed.symptoms
        );
        let contracts = if let Some(bind) = parsed.ast.defs.get(&intern("reserve")) {
            &bind.payload_postconditions
        } else {
            let ast::DeclareValue::Has(members) = &parsed.ast.tags[&intern("Allocator")].value
            else {
                panic!();
            };
            let ast::HasMember::Function(method) = &members[0] else {
                panic!();
            };
            &method.payload_postconditions
        };
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].name.as_str(), "storage");
        assert!(matches!(
            contracts[0].proposition,
            ast::ProofProposition::And(..)
        ));
    }
}
