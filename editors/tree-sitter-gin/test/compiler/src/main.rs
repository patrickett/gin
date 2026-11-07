use serde_json::{Value, json};
use lexer::{Lexer, TokenKind};
use parser::parse_strict;
use std::io;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--keywords") {
        let keywords: Vec<_> = TokenKind::KEYWORDS
            .iter()
            .filter_map(|kind| kind.keyword())
            .collect();
        serde_json::to_writer(io::stdout(), &keywords).unwrap();
        return;
    }
    let cases: Vec<Value> = serde_json::from_reader(io::stdin()).unwrap();
    let results: Vec<_> = cases
        .iter()
        .map(|case| {
            let source = case["source"].as_str().unwrap();
            let mut lexer = Lexer::new(source);
            let mut tokens = Vec::new();
            while let Some(token) = lexer.next_raw() {
                let start = token.span.start() as usize;
                let end = token.span.end() as usize;
                tokens.push(json!({
                    "kind": format!("{:?}", token.kind),
                    "start": source[..start].encode_utf16().count(),
                    "end": source[..end].encode_utf16().count(),
                }));
            }
            let diagnostics = parse_strict(source)
                .err()
                .unwrap_or_default()
                .into_iter()
                .map(|diagnostic| format!("{}: {}", diagnostic.code.slug(), diagnostic.message))
                .collect::<Vec<_>>();
            json!({ "tokens": tokens, "diagnostics": diagnostics })
        })
        .collect();
    serde_json::to_writer(io::stdout(), &results).unwrap();
}
