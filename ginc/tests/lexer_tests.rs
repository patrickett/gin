use ginc::lexer::{GinLexer, Token};

#[test]
fn test_keywords() {
    let src = "use if else for as is in of or and has when then loop continue break return private";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::Use));
    assert!(matches!(tokens[1], Token::If));
    assert!(matches!(tokens[2], Token::Else));
    assert!(matches!(tokens[3], Token::For));
    assert!(matches!(tokens[4], Token::As));
    assert!(matches!(tokens[5], Token::Is));
    assert!(matches!(tokens[6], Token::In));
    assert!(matches!(tokens[7], Token::Of));
    assert!(matches!(tokens[8], Token::Or));
    assert!(matches!(tokens[9], Token::And));
    assert!(matches!(tokens[10], Token::Has));
    assert!(matches!(tokens[11], Token::When));
    assert!(matches!(tokens[12], Token::Then));
    assert!(matches!(tokens[13], Token::Loop));
    assert!(matches!(tokens[14], Token::Continue));
    assert!(matches!(tokens[15], Token::Break));
    assert!(matches!(tokens[16], Token::Return));
    assert!(matches!(tokens[17], Token::Private));
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
    let src = "== /= <= >= = < > + - * / ^ ~ \\";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::EqEq));
    assert!(matches!(tokens[1], Token::NotEq));
    assert!(matches!(tokens[2], Token::LessEq));
    assert!(matches!(tokens[3], Token::GreaterEq));
    assert!(matches!(tokens[4], Token::Eq));
    assert!(matches!(tokens[5], Token::Less));
    assert!(matches!(tokens[6], Token::Greater));
    assert!(matches!(tokens[7], Token::Plus));
    assert!(matches!(tokens[8], Token::Minus));
    assert!(matches!(tokens[9], Token::Star));
    assert!(matches!(tokens[10], Token::Slash));
    assert!(matches!(tokens[11], Token::Caret));
    assert!(matches!(tokens[12], Token::Tilde));
    assert!(matches!(tokens[13], Token::SlashOr));
}

#[test]
fn test_punctuation() {
    let src = "( ) [ ] { } , . : ; ...";

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
    assert!(matches!(tokens[10], Token::Infer));
}

#[test]
fn test_comments() {
    let src = "-- this is a comment\n--- this is a doc comment";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0], Token::Newline));
    assert!(matches!(tokens[1], Token::DocComment(_)));
}

#[test]
fn test_indentation() {
    let src = "foo:\n    bar\n  baz";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // Should have: foo, does, :, newline, indent, bar, newline, dedent, baz
    assert!(matches!(tokens[0], Token::Id(_)));
    assert!(matches!(tokens[1], Token::Colon));

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
    let src = "--- Currently just a marker trait\nSized ()";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::DocComment(_)));
    assert!(matches!(tokens[1], Token::Newline));
    assert!(matches!(tokens[2], Token::Tag(_)));
    assert!(matches!(tokens[3], Token::ParenOpen));
    assert!(matches!(tokens[4], Token::ParenClose));
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
fn test_unicode_string_literals() {
    // Multi-byte UTF-8 characters inside single-quoted strings
    let src = "'héllo' 'こんにちは'";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(matches!(tokens[0], Token::String(_)));
    assert_eq!(
        if let Token::String(s) = &tokens[0] {
            *s
        } else {
            ""
        },
        "héllo"
    );

    assert!(matches!(tokens[1], Token::String(_)));
    assert_eq!(
        if let Token::String(s) = &tokens[1] {
            *s
        } else {
            ""
        },
        "こんにちは"
    );
}

#[test]
fn test_unicode_format_strings() {
    // Multi-byte UTF-8 characters inside double-quoted format strings
    let src = r#""héllo" "こんにちは""#;

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // "héllo" → [FormatStringDelim, FormatStringText("héllo"), FormatStringDelim]
    assert_eq!(tokens[0], Token::FormatStringDelim);
    assert_eq!(tokens[1], Token::FormatStringText("héllo"));
    assert_eq!(tokens[2], Token::FormatStringDelim);

    // "こんにちは" → [FormatStringDelim, FormatStringText("こんにちは"), FormatStringDelim]
    assert_eq!(tokens[3], Token::FormatStringDelim);
    assert_eq!(tokens[4], Token::FormatStringText("こんにちは"));
    assert_eq!(tokens[5], Token::FormatStringDelim);
}

#[test]
fn test_unicode_comments() {
    let src = "-- héllo wörld";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert!(tokens.is_empty());
}

#[test]
fn test_unicode_tags_rejected() {
    // Non-ASCII uppercase letters are no longer valid tag starts (ASCII only)
    let src = "Ångström Élève";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // Neither should be recognized as tags — only ASCII [A-Z] starts a tag
    assert!(tokens.iter().all(|t| !matches!(t, Token::Tag(_))));
}

#[test]
fn test_unicode_identifiers_rejected() {
    // Non-ASCII lowercase letters are no longer valid identifiers (ASCII only)
    let src = "café αλφα";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // "caf" is a valid ASCII id, but "é" breaks it; "αλφα" is not recognized at all
    assert!(tokens.iter().all(|t| {
        if let Token::Id(s) = t {
            s.is_ascii()
        } else {
            true
        }
    }));
}

#[test]
fn test_unicode_non_letter_not_id() {
    // `hello_世界`: `世` and `界` are Other_Letter (Lo), not Lowercase_Letter (Ll),
    // so they cannot form a snake_case segment. Only `hello` is tokenised as Id;
    // the remaining characters are silently skipped as unrecognised.
    let src = "hello_世界";

    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // First (and only) real token is the `hello` identifier
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0], Token::Id(_)));
    assert_eq!(
        if let Token::Id(s) = &tokens[0] {
            *s
        } else {
            ""
        },
        "hello"
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
            "non-empty"
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
            "non-empty"
        },
        ""
    );
}

#[test]
fn test_format_string_interpolation() {
    let src = r#""hello (name)""#;
    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert_eq!(
        tokens,
        vec![
            Token::FormatStringDelim,
            Token::FormatStringText("hello "),
            Token::FormatInterpStart,
            Token::Id("name"),
            Token::FormatInterpEnd,
            Token::FormatStringDelim,
        ]
    );
    assert!(lexer.errors.is_empty());
}

#[test]
fn test_format_string_nested_parens() {
    let src = r#""result (foo(x, y))""#;
    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert_eq!(
        tokens,
        vec![
            Token::FormatStringDelim,
            Token::FormatStringText("result "),
            Token::FormatInterpStart,
            Token::Id("foo"),
            Token::ParenOpen,
            Token::Id("x"),
            Token::Comma,
            Token::Id("y"),
            Token::ParenClose,
            Token::FormatInterpEnd,
            Token::FormatStringDelim,
        ]
    );
    assert!(lexer.errors.is_empty());
}

#[test]
fn test_format_string_unterminated_interp() {
    let src = r#""hello (name"#;
    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    // Should contain UnterminatedFormatString somewhere
    assert!(
        tokens
            .iter()
            .any(|t| matches!(t, Token::UnterminatedFormatString))
    );
    assert!(!lexer.errors.is_empty());
}

#[test]
fn test_format_string_empty() {
    let src = r#""""#;
    let mut lexer = GinLexer::new(src);
    let tokens: Vec<_> = lexer.by_ref().map(|(tok, _)| tok).collect();

    assert_eq!(
        tokens,
        vec![Token::FormatStringDelim, Token::FormatStringDelim,]
    );
    assert!(lexer.errors.is_empty());
}
