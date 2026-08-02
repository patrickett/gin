use diagnostic::Category;
use lexer::{Lexer, Token};
use parser::query::SourceParseExt;

mod support;
use support::*;

#[test]
fn lower_start_camel_case_lexes_as_one_identifier() {
    let tokens: Vec<_> = Lexer::new("regConstraint(reg Register) String: ''")
        .map(|(token, _)| token)
        .collect();

    assert!(matches!(tokens.first(), Some(Token::Id("regConstraint"))));
}

#[test]
fn camel_case_bind_name_parses_with_snake_case_warning() {
    let out = "regConstraint(reg Register) String: ''\n".parse_source_full();

    assert!(
        out.ast.defs.contains_key(&intern("regConstraint")),
        "camelCase bind should still parse as a bind"
    );
    assert!(
        !out.symptoms
            .iter()
            .any(|diag| diag.category == Category::Flaw),
        "camelCase bind name should not produce parse flaws: {:?}",
        out.symptoms
    );
    assert!(
        out.ast.parse_warnings.iter().any(|diag| {
            diag.code.slug() == "parse-bind-name-not-snake-case"
                && diag.message.contains("regConstraint")
        }),
        "expected snake_case warning, got {:?}",
        out.ast.parse_warnings
    );
}
