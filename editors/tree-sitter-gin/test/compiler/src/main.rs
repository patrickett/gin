use lexer::Lexer;
use parser::query::SourceParseExt;
use serde_json::{Value, json};
use std::io;

fn main() {
    let cases: Vec<Value> = serde_json::from_reader(io::stdin()).unwrap();
    let results: Vec<_> = cases
        .iter()
        .map(|case| {
            let source = case["source"].as_str().unwrap();
            let mut lexer = Lexer::new(source);
            let mut tokens = Vec::new();
            while let Some((token, id)) = lexer.next_raw() {
                let span = lexer.get_span(id);
                let name = format!("{token:?}");
                tokens.push(json!({
                    "kind": name.split('(').next().unwrap(),
                    "start": source[..span.start()].encode_utf16().count(),
                    "end": source[..span.end()].encode_utf16().count(),
                }));
            }
            let diagnostics = if case.get("valid").is_some() {
                source
                    .parse_source_full()
                    .symptoms
                    .iter()
                    .filter(|d| d.category.as_str() == "flaw")
                    .map(|d| format!("{}: {}", d.code.slug(), d.message))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            json!({ "tokens": tokens, "diagnostics": diagnostics })
        })
        .collect();
    serde_json::to_writer(io::stdout(), &results).unwrap();
}
