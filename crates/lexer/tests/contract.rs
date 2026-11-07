use lexer::{Lexer, Token, TokenKind};

#[test]
fn tokens_are_only_kinds_and_source_ranges() {
    fn copy_eq<T: Copy + Eq>() {}
    copy_eq::<Token>();
    copy_eq::<TokenKind>();
    assert!(std::mem::size_of::<Token>() <= 16);
    let detached: Token = {
        let temporary = std::string::String::from("detached");
        Lexer::new(&temporary).next().unwrap()
    };
    assert_eq!(detached.kind, TokenKind::Id);
    let source = "first second 'text'";
    let tokens: Vec<Token> = Lexer::new(source).collect();
    assert_eq!(tokens[0].kind, TokenKind::Id);
    assert_eq!(tokens[1].kind, TokenKind::Id);
    assert_eq!(tokens[0].span.extract(source), "first");
    assert_eq!(tokens[1].span.extract(source), "second");
    assert_eq!(tokens[2].span.extract(source), "'text'");
}

fn scan(source: &str) -> Vec<(TokenKind, &str)> {
    Lexer::new(source)
        .map(|token| (token.kind, token.span.extract(source)))
        .collect()
}

#[test]
fn numeric_classification_does_not_convert_or_bound_values() {
    use TokenKind::*;
    for (source, kind) in [
        ("1_000", Int),
        ("0xA_B", Int),
        ("0Xff", Int),
        ("1_000.2_5", Float),
        ("1__2", MalformedInt),
        ("12_", MalformedInt),
        ("0xff__", MalformedInt),
        ("1_.25", MalformedFloat),
        ("1.2_", MalformedFloat),
        ("1.2__3", MalformedFloat),
    ] {
        assert_eq!(scan(source), [(kind, source)]);
    }
    for source in [
        "9".repeat(10_000),
        format!("0x{}", "f".repeat(10_000)),
        format!("{}.0", "9".repeat(10_000)),
        format!("0.{}1", "0".repeat(10_000)),
    ] {
        let expected = if source.contains('.') { Float } else { Int };
        assert_eq!(scan(&source), [(expected, source.as_str())]);
    }
    assert_eq!(
        scan("0x_1 0x 1. 1._2 1e2"),
        [
            (Int, "0"),
            (Id, "x_1"),
            (Int, "0"),
            (Id, "x"),
            (Int, "1"),
            (Dot, "."),
            (Int, "1"),
            (Dot, "."),
            (Id, "_2"),
            (Int, "1"),
            (Id, "e2"),
        ]
    );
}

#[test]
fn all_keywords_and_identifier_prefixes() {
    for &kind in TokenKind::KEYWORDS {
        let spelling = kind.keyword().unwrap();
        assert_eq!(scan(spelling), [(kind, spelling)]);
        let source = format!("{spelling}value");
        let expected = if spelling == "Self" {
            TokenKind::Tag
        } else {
            TokenKind::Id
        };
        assert_eq!(scan(&source), [(expected, source.as_str())]);
    }
    assert_eq!(
        scan("Tag_name _name"),
        [
            (TokenKind::Tag, "Tag"),
            (TokenKind::Id, "_name"),
            (TokenKind::Id, "_name"),
        ]
    );
}

#[test]
fn all_operators_and_longest_matches() {
    use TokenKind::*;
    for (source, kind) in [
        ("+", Plus),
        ("-", Minus),
        ("*", Star),
        ("/", Slash),
        ("%", Percent),
        ("&", Ampersand),
        ("|", Pipe),
        ("^", Caret),
        ("\\", SlashOr),
        ("<", Less),
        (">", Greater),
        ("=", Eq),
        ("<<", ShiftLeft),
        (">>", ShiftRight),
        ("<=", LessEq),
        (">=", GreaterEq),
        ("<-", ArrowLeft),
        ("==", EqEq),
        ("/=", NotEq),
        (":", Colon),
        ("::", ColonColon),
        (":=", ColonEq),
        (";", ColonSemi),
        (".", Dot),
        ("...", Infer),
        ("(", ParenOpen),
        (")", ParenClose),
        ("[", BracketOpen),
        ("]", BracketClose),
        ("{", CurlyOpen),
        ("}", CurlyClose),
        (",", Comma),
        ("@", At),
        ("#", Pound),
        ("?", Question),
        ("\"", FormatStringDelim),
    ] {
        assert_eq!(scan(source), [(kind, source)]);
    }
    assert_eq!(
        scan("<<< .... ::= :== ->"),
        [
            (ShiftLeft, "<<"),
            (Less, "<"),
            (Infer, "..."),
            (Dot, "."),
            (ColonColon, "::"),
            (Eq, "="),
            (ColonEq, ":="),
            (Eq, "="),
            (Minus, "-"),
            (Greater, ">"),
        ]
    );
}

#[test]
fn exact_string_failure_and_unicode_scalars() {
    use TokenKind::*;
    assert_eq!(
        scan("'世界\nnext '"),
        [
            (UnterminatedString, "'世界"),
            (Newline, "\n"),
            (Id, "next"),
            (UnterminatedString, "'"),
        ]
    );
    assert_eq!(
        scan("'' 'a\\b' 'é\r'"),
        [(String, "''"), (String, "'a\\b'"), (String, "'é\r'"),]
    );
    assert_eq!(
        scan("é\r\n💡~中"),
        [
            (Unknown, "é"),
            (Unknown, "\r"),
            (Newline, "\n"),
            (Unknown, "💡"),
            (Unknown, "~"),
            (Unknown, "中"),
        ]
    );
}

#[test]
fn comments_newlines_layout_and_format_remain_raw() {
    use TokenKind::*;
    assert_eq!(
        scan("-- héllo\r\n\n--- doc\n--| module\n"),
        [
            (Comment, "-- héllo\r"),
            (Newline, "\n"),
            (Newline, "\n"),
            (DocComment, "--- doc"),
            (Newline, "\n"),
            (ModuleDocComment, "--| module"),
            (Newline, "\n"),
        ]
    );
    let source = format!("root\n{}child\n\tend\n", " ".repeat(70_000));
    assert_eq!(
        scan(&source),
        [
            (Id, "root"),
            (Newline, "\n"),
            (Id, "child"),
            (Newline, "\n"),
            (Id, "end"),
            (Newline, "\n"),
        ]
    );
    let mut lexer = Lexer::new("\"\\(value\r");
    let first = lexer.next_raw().unwrap();
    assert_eq!(first.kind, FormatStringDelim);
    assert_eq!(first.span.to_range(), 0..1);
    assert_eq!(lexer.next().unwrap().kind, SlashOr);
    assert_eq!(lexer.next().unwrap().kind, ParenOpen);
    assert_eq!(lexer.next().unwrap().kind, Id);
    assert_eq!(lexer.next().unwrap().kind, Unknown);
}

#[test]
fn every_unicode_prefix_advances_and_eof_is_stable() {
    let source = "é -- 中\n'世界'\n\"\\(value)\" 🦀 1__2 0xff_ 'bad\n \t";
    for end in (0..=source.len()).filter(|&end| source.is_char_boundary(end)) {
        let prefix = &source[..end];
        let mut lexer = Lexer::new(prefix);
        let expected: Vec<_> = Lexer::new(prefix).collect();
        let mut previous_end = 0;
        let mut actual = Vec::new();
        for _ in 0..=prefix.len() {
            let Some(token) = lexer.next_raw() else { break };
            assert!(token.span.start() >= previous_end);
            assert!(token.span.end() > previous_end);
            assert!(!token.span.is_empty());
            assert!(prefix.get(token.span.to_range()).is_some());
            previous_end = token.span.end();
            actual.push(token);
        }
        assert_eq!(actual, expected);
        assert!(lexer.next_raw().is_none());
        assert!(lexer.next().is_none());
        assert!(lexer.next_raw().is_none());
    }
}

/// A consumer that reads a format string's text itself steps the scanner past it.
#[test]
fn skip_to_resumes_after_text_the_consumer_read() {
    let source = "\"it's\" x";
    let mut lexer = Lexer::new(source);
    assert_eq!(lexer.next_raw().unwrap().kind, TokenKind::FormatStringDelim);
    lexer.skip_to(6);
    let next = lexer.next_raw().unwrap();
    assert_eq!((next.kind, next.span.extract(source)), (TokenKind::Id, "x"));
}
