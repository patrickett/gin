use ginc::frontend::lexer::{GinLexer, Token};

#[test]
fn test_keywords() {
    let src = "def use if else for where as do is in of or and has when then does from loop continue break return private public alias macro needs derives optional required";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::Def));
    assert!(matches!(tokens[1], Token::Use));
    assert!(matches!(tokens[2], Token::If));
    assert!(matches!(tokens[3], Token::Else));
    assert!(matches!(tokens[4], Token::For));
    assert!(matches!(tokens[5], Token::Where));
    assert!(matches!(tokens[6], Token::As));
    assert!(matches!(tokens[7], Token::Do));
    assert!(matches!(tokens[8], Token::Is));
    assert!(matches!(tokens[9], Token::In));
    assert!(matches!(tokens[10], Token::Of));
    assert!(matches!(tokens[11], Token::Or));
    assert!(matches!(tokens[12], Token::And));
    assert!(matches!(tokens[13], Token::Has));
    assert!(matches!(tokens[14], Token::When));
    assert!(matches!(tokens[15], Token::Then));
    assert!(matches!(tokens[16], Token::Does));
    assert!(matches!(tokens[17], Token::From));
    assert!(matches!(tokens[18], Token::Loop));
    assert!(matches!(tokens[19], Token::Continue));
    assert!(matches!(tokens[20], Token::Break));
    assert!(matches!(tokens[21], Token::Return));
    assert!(matches!(tokens[22], Token::Private));
    assert!(matches!(tokens[23], Token::Public));
    assert!(matches!(tokens[24], Token::Alias));
    assert!(matches!(tokens[25], Token::Macro));
    assert!(matches!(tokens[26], Token::Needs));
    assert!(matches!(tokens[27], Token::Derives));
    assert!(matches!(tokens[28], Token::Optional));
    assert!(matches!(tokens[29], Token::Required));
}

#[test]
fn test_identifiers() {
    let src = "foo bar baz hello_world";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::Id(_)));
    assert!(matches!(tokens[1], Token::Id(_)));
    assert!(matches!(tokens[2], Token::Id(_)));
    assert!(matches!(tokens[3], Token::Id(_)));
}

#[test]
fn test_tags() {
    let src = "User Error HTTPRequest ServerState";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::Tag(_)));
    assert!(matches!(tokens[1], Token::Tag(_)));
    assert!(matches!(tokens[2], Token::Tag(_)));
    assert!(matches!(tokens[3], Token::Tag(_)));
}

#[test]
fn test_numbers() {
    let src = "42 3.14 0 999";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::Int(_)));
    assert_eq!(
        if let Token::Int(v) = &tokens[0] {
            *v
        } else {
            0
        },
        42
    );

    assert!(matches!(tokens[1], Token::Float(_)));

    assert!(matches!(tokens[2], Token::Int(_)));
    assert_eq!(
        if let Token::Int(v) = &tokens[2] {
            *v
        } else {
            0
        },
        0
    );

    assert!(matches!(tokens[3], Token::Int(_)));
}

#[test]
fn test_operators() {
    let src = "== != <= >= = < > + - * / | ^ ~ ::=";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::EqEq));
    assert!(matches!(tokens[1], Token::NotEqual));
    assert!(matches!(tokens[2], Token::LessEq));
    assert!(matches!(tokens[3], Token::GreaterEq));
    assert!(matches!(tokens[4], Token::Equals));
    assert!(matches!(tokens[5], Token::Less));
    assert!(matches!(tokens[6], Token::Greater));
    assert!(matches!(tokens[7], Token::Plus));
    assert!(matches!(tokens[8], Token::Minus));
    assert!(matches!(tokens[9], Token::Star));
    assert!(matches!(tokens[10], Token::Slash));
    assert!(matches!(tokens[11], Token::Bar));
    assert!(matches!(tokens[12], Token::Caret));
    assert!(matches!(tokens[13], Token::Tilde));
    assert!(matches!(tokens[14], Token::IsReplacedBy));
    // assert!(matches!(tokens[15], Token::Colon));
}

#[test]
fn test_punctuation() {
    let src = "( ) [ ] { } , . : ; .. ...";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::ParenOpen));
    assert!(matches!(tokens[1], Token::ParenClose));
    assert!(matches!(tokens[2], Token::BracketOpen));
    assert!(matches!(tokens[3], Token::BracketClose));
    assert!(matches!(tokens[4], Token::CurlyOpen));
    assert!(matches!(tokens[5], Token::CurlyClose));
    assert!(matches!(tokens[6], Token::Comma));
    assert!(matches!(tokens[7], Token::Dot));
    assert!(matches!(tokens[8], Token::Colon));
    assert!(matches!(tokens[9], Token::ColonSemi));
    assert!(matches!(tokens[10], Token::DotDot));
    assert!(matches!(tokens[11], Token::Ellipsis));
}

#[test]
fn test_comments() {
    let src = "-- this is a comment\n--- this is a doc comment";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // Comments should be recognized (note newline between them)
    assert!(matches!(tokens[0], Token::Comment(_)));
    assert!(matches!(tokens[1], Token::Newline));
    assert!(matches!(tokens[2], Token::DocComment(_)));
}

#[test]
fn test_indentation() {
    let src = "def foo():\n    bar\n  baz";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // Should have: def, foo, (, ), :, newline, indent, bar, newline, dedent, baz
    assert!(matches!(tokens[0], Token::Def));
    assert!(matches!(tokens[1], Token::Id(_)));
    assert!(matches!(tokens[2], Token::ParenOpen));
    assert!(matches!(tokens[3], Token::ParenClose));
    assert!(matches!(tokens[4], Token::Colon));

    // After newline should come indent
    let mut found_indent = false;
    for tok in &tokens {
        if matches!(tok, Token::Indent) {
            found_indent = true;
            break;
        }
    }
    assert!(found_indent);
}

#[test]
fn test_play() {
    let src = "--- Currently just a marker trait\nSized does ()";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();
    println!("{:#?}", tokens);

    assert!(matches!(tokens[0], Token::DocComment(_)));
    assert!(matches!(tokens[1], Token::Newline));
    assert!(matches!(tokens[2], Token::Tag(_)));
    assert!(matches!(tokens[3], Token::Does));
    assert!(matches!(tokens[4], Token::ParenOpen));
    assert!(matches!(tokens[5], Token::ParenClose));
}

#[test]
fn test_string_literal() {
    let src = "'foo' 'bar' 'baz'";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::String(_)));
    assert_eq!(
        if let Token::String(s) = &tokens[0] {
            *s
        } else {
            ""
        },
        "foo"
    );

    assert!(matches!(tokens[1], Token::String(_)));
    assert_eq!(
        if let Token::String(s) = &tokens[1] {
            *s
        } else {
            ""
        },
        "bar"
    );

    assert!(matches!(tokens[2], Token::String(_)));
    assert_eq!(
        if let Token::String(s) = &tokens[2] {
            *s
        } else {
            ""
        },
        "baz"
    );
}

#[test]
fn test_unterminated_string_with_content() {
    let src = "x: 'bar\nz: 'baz";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // x : UnterminatedString("bar") Newline z : UnterminatedString("baz")
    assert!(matches!(tokens[0], Token::Id(_)));
    assert!(matches!(tokens[1], Token::Colon));
    assert!(matches!(tokens[2], Token::UnterminatedString(_)));
    assert_eq!(
        if let Token::UnterminatedString(s) = &tokens[2] {
            *s
        } else {
            ""
        },
        "bar"
    );
    assert!(matches!(tokens[3], Token::Newline));
    assert!(matches!(tokens[4], Token::Id(_)));
    assert!(matches!(tokens[5], Token::Colon));
    assert!(matches!(tokens[6], Token::UnterminatedString(_)));
    assert_eq!(
        if let Token::UnterminatedString(s) = &tokens[6] {
            *s
        } else {
            ""
        },
        "baz"
    );
}

#[test]
fn test_unterminated_string_lone_quote() {
    let src = "y: '\nx: '";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // y : UnterminatedString("") Newline x : UnterminatedString("")
    assert!(matches!(tokens[0], Token::Id(_)));
    assert!(matches!(tokens[1], Token::Colon));
    assert!(matches!(tokens[2], Token::UnterminatedString(_)));
    assert_eq!(
        if let Token::UnterminatedString(s) = &tokens[2] {
            *s
        } else {
            ""
        },
        ""
    );
    assert!(matches!(tokens[3], Token::Newline));
    assert!(matches!(tokens[4], Token::Id(_)));
    assert!(matches!(tokens[5], Token::Colon));
    assert!(matches!(tokens[6], Token::UnterminatedString(_)));
    assert_eq!(
        if let Token::UnterminatedString(s) = &tokens[6] {
            *s
        } else {
            ""
        },
        ""
    );
}
