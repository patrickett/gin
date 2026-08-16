use parser::cursor::TokenCursor;

mod support;
use support::intern;

#[test]
fn parses_explicit_anonymous_result_surface() {
    let ast = TokenCursor::parse_source("choose(ref value Int) True or False: True");
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
