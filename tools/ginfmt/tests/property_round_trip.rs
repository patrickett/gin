use lexer::Lexer;

const INPUT_LIMIT: usize = 4096;

fn check_lexer(source: &str) {
    assert!(source.len() <= INPUT_LIMIT);
    let mut lexer = Lexer::new(source);
    let mut previous_end = 0;
    let mut count = 0;
    while let Some(token) = lexer.next_raw() {
        let range = token.span.to_range();
        assert!(range.start >= previous_end);
        assert!(range.start <= range.end);
        assert!(range.end <= source.len());
        assert!(source.is_char_boundary(range.start));
        assert!(source.is_char_boundary(range.end));
        if !range.is_empty() {
            assert!(range.end > previous_end);
        }
        previous_end = range.end;
        count += 1;
        assert!(count <= source.len().saturating_add(1));
    }
}

fn check_parser_total(source: &str) {
    let _ = parser::parse(source);
    if let Ok(formatted) = ginfmt::format(source) {
        let parsed = parser::parse(&formatted).expect("formatted arbitrary input reparses");
        assert!(!parsed.has_errors());
        ginfmt::verify::check_semantics(source, &formatted).unwrap();
        ginfmt::verify::check_comments(source, &formatted).unwrap();
        assert_eq!(ginfmt::format(&formatted).unwrap(), formatted);
    }
}

fn check_formattable(source: &str) {
    let formatted = ginfmt::format(source).expect("valid generated source formats");
    ginfmt::verify::check_semantics(source, &formatted).unwrap();
    ginfmt::verify::check_comments(source, &formatted).unwrap();
    let parsed = parser::parse(&formatted).expect("formatted output parses without budget failure");
    assert!(
        !parsed.has_errors(),
        "formatted output has parser flaws: {formatted:?}"
    );
    assert_eq!(
        ginfmt::format(&formatted).expect("formatted output formats again"),
        formatted
    );
}

#[test]
fn bounded_arbitrary_utf8_is_total_and_lexical_spans_are_safe() {
    // Exhaustively enumerate short strings over malformed syntax and multi-byte scalars.
    let alphabet = ["a", " ", "\n", "\"", "(", ")", ":", "💣", "\u{fffd}"];
    let mut states = vec![String::new()];
    for _ in 0..4 {
        let previous = states.clone();
        for prefix in previous {
            for piece in alphabet {
                let mut input = prefix.clone();
                input.push_str(piece);
                if input.len() <= INPUT_LIMIT {
                    check_lexer(&input);
                    check_parser_total(&input);
                    states.push(input);
                }
            }
        }
    }
    for repeated in ["(", ")", "\"", "💣", "\n", "a"] {
        let input = repeated.repeat(INPUT_LIMIT / repeated.len());
        check_lexer(&input);
        check_parser_total(&input);
    }
}

#[test]
fn semantic_oracle_rejects_deletion_even_when_empty_output_is_stable() {
    let source = "answer := 7\n";
    assert_eq!(ginfmt::format("").unwrap(), "");
    assert!(ginfmt::verify::check_semantics(source, "").is_err());
}

#[test]
fn generated_valid_sources_format_and_reparse() {
    let names = ["answer", "count", "value", "result"];
    for name in names {
        for value in 0..32 {
            let source = format!("{name} := {value}\n");
            check_lexer(&source);
            check_formattable(&source);
        }
    }
    for value in 0..32 {
        let source = format!(
            "run:\n    item: {value}\n    when item < 16 then\n        return item\n    return 0\n"
        );
        check_lexer(&source);
        check_formattable(&source);
    }
}
