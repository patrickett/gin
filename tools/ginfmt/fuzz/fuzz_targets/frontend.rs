#![no_main]

use lexer::Lexer;
use libfuzzer_sys::fuzz_target;

const INPUT_LIMIT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > INPUT_LIMIT {
        return;
    }
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let mut lexer = Lexer::new(source);
    let mut previous_end = 0;
    let mut token_count = 0;
    while let Some(token) = lexer.next_raw() {
        let span = token.span;
        assert!(span.start() >= previous_end);
        assert!(span.start() <= span.end());
        assert!(span.end() <= source.len());
        assert!(source.is_char_boundary(span.start()));
        assert!(source.is_char_boundary(span.end()));
        if span.start() != span.end() {
            assert!(span.end() > previous_end);
        }
        previous_end = span.end();
        token_count += 1;
        assert!(token_count <= source.len().saturating_add(1));
    }

    let _ = parser::parse(source);
    if let Ok(formatted) = ginfmt::format(source) {
        ginfmt::verify::check_semantics(source, &formatted).unwrap();
        ginfmt::verify::check_comments(source, &formatted).unwrap();
        let parsed = parser::parse(&formatted).expect("formatter output parse completes");
        assert!(!parsed.has_errors());
        assert_eq!(
            ginfmt::format(&formatted).expect("formatter output is stable"),
            formatted
        );
    }
});
