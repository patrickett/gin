use lexer::{Lexer, Token};

#[test]
fn unexpected_unicode_diagnostics_cover_complete_characters() {
    let source = "é 中 🦀 tail";
    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.by_ref().map(|(token, _)| token).collect();
    assert_eq!(tokens, vec![Token::Id("tail")]);
    assert_eq!(lexer.errors.len(), 3);
    for (diagnostic, character) in lexer.errors.iter().zip(["é", "中", "🦀"]) {
        assert_eq!(diagnostic.error_code(), "lex-unexpected-character");
        assert_eq!(source.get(diagnostic.span.to_range()), Some(character));
    }
}

#[test]
fn escaped_unicode_preserves_complete_characters_and_spans() {
    for character in ['é', '中', '🦀'] {
        let source = format!("\"\\{character}\" tail");
        let mut lexer = Lexer::new(&source);
        let tokens: Vec<_> = lexer.by_ref().collect();
        assert!(lexer.errors.is_empty(), "{:?}", lexer.errors);
        assert_eq!(tokens.len(), 4);
        assert_eq!(
            tokens[1].0,
            Token::FormatStringText(&source[1..2 + character.len_utf8()])
        );
        assert_eq!(tokens[3].0, Token::Id("tail"));
        for (_, span) in tokens {
            let span = lexer.get_span(span);
            assert!(source.is_char_boundary(span.start()));
            assert!(source.is_char_boundary(span.end()));
        }
    }
}

#[test]
fn excessive_indentation_reports_a_diagnostic_and_recovers() {
    for whitespace in [" ".repeat(65_536), "\t".repeat(16_384)] {
        let source = format!("root\n{whitespace}nested\ntail");
        let mut lexer = Lexer::new(&source);
        let tokens: Vec<_> = lexer.by_ref().map(|(token, _)| token).collect();
        assert_eq!(tokens.last(), Some(&Token::Id("tail")));
        assert_eq!(
            tokens
                .iter()
                .filter(|token| **token == Token::Indent)
                .count(),
            1
        );
        assert_eq!(
            tokens
                .iter()
                .filter(|token| **token == Token::Dedent)
                .count(),
            1
        );
        assert_eq!(lexer.errors.len(), 1, "{:?}", lexer.errors);
        assert_eq!(lexer.errors[0].error_code(), "lex-overflow-indent");
    }
}

#[test]
fn iterator_size_hint_bounds_hold_at_every_token_boundary() {
    for source in [
        "++++",
        "-- a long comment without emitted tokens",
        "\"text\\nmore\"",
        "\"\\(a + b)\"",
        "root\n    child\n        leaf",
        "root\n    child\n        leaf\ntail",
    ] {
        let token_count = Lexer::new(source).count();
        let mut lexer = Lexer::new(source);
        for consumed in 0..=token_count {
            let remaining = token_count - consumed;
            let (lower, upper) = lexer.size_hint();
            assert!(
                lower <= remaining,
                "{source:?}, consumed {consumed}: {lower} > {remaining}"
            );
            assert!(
                upper.is_none_or(|upper| remaining <= upper),
                "{source:?}, consumed {consumed}: {upper:?} < {remaining}"
            );
            lexer.next();
        }
    }
}

#[test]
fn truncated_unicode_and_interpolation_inputs_keep_valid_spans() {
    for source in [
        "\"text\\é\\中\\🦀\\(call(3))\"\ntail",
        "root\n    \"\\🦀\"\n        child\ntail",
        "é -- 中\n\"\\🦀",
    ] {
        for end in (0..=source.len()).filter(|end| source.is_char_boundary(*end)) {
            let prefix = &source[..end];
            let mut lexer = Lexer::new(prefix);
            let tokens: Vec<_> = lexer.by_ref().collect();
            for (_, span) in tokens {
                let span = lexer.get_span(span);
                assert!(
                    prefix.get(span.to_range()).is_some(),
                    "{prefix:?}: {span:?}"
                );
            }
            for diagnostic in lexer.errors {
                assert!(
                    prefix.get(diagnostic.span.to_range()).is_some(),
                    "{prefix:?}: {diagnostic:?}"
                );
            }
        }
    }
}
