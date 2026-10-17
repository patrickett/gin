use lexer::{Lexer, Token};

#[test]
fn dedent_to_unseen_column_closes_the_nearest_shallower_level() {
    let source = "root\n    child\n  sibling\n";
    let mut lexer = Lexer::new(source);
    let tokens = lexer.by_ref().map(|(token, _)| token).collect::<Vec<_>>();

    assert_eq!(
        tokens,
        [
            Token::Id("root"),
            Token::Newline,
            Token::Indent,
            Token::Id("child"),
            Token::Newline,
            Token::Dedent,
            Token::Id("sibling"),
            Token::Newline,
        ]
    );
    assert!(lexer.errors.is_empty(), "{:?}", lexer.errors);
}
