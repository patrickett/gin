use parser::cursor::TokenCursor;

use lexer::{Lexer, Token};

fn parse_when_source(source: &str) -> Option<ast::WhenExpr> {
    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer
        .by_ref()
        .filter(|(t, _)| !matches!(t, Token::Comment(_)))
        .collect();
    let mut span_table = lexer.take_span_table();
    let mut cursor = TokenCursor::new(&tokens, &mut span_table);
    cursor.parse_when_expr(|c: &mut TokenCursor| c.parse_expression())
}

#[test]
fn when_is_multiline_arms_parse() {
    let source = "when target.arch is\n    'x86_64' then BigInt\n    else Int";
    assert!(
        parse_when_source(source).is_some(),
        "multiline is-when should parse"
    );
}

#[test]
fn when_is_single_line_parses() {
    let source = "when target.arch is 'x86_64' then BigInt else Int";
    assert!(parse_when_source(source).is_some());
}
