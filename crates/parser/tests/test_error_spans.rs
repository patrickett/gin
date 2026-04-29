// Test to verify span information is available in parser errors
// Uses the handwritten parser directly to check error spans without Salsa overhead.

use lexer::Lexer;
use parser::expr;

#[test]
fn test_error_spans() {
    let source = "x + "; // Incomplete expression

    let mut lexer = Lexer::new(source);
    let tokens: Vec<_> = lexer.by_ref().collect();
    let mut span_table = lexer.take_span_table();
    let (output, errors) = expr::parse_tokens_with_errors(&tokens, &mut span_table);

    if errors.is_empty() {
        eprintln!("Unexpected success: {:?}", output);
        panic!("Expected parse errors for incomplete expression");
    } else {
        eprintln!("Got {} error(s) as expected:", errors.len());
        for error in &errors {
            let span = span_table.get(error.span);
            eprintln!("    Error: {}", error.message);
            eprintln!("      Span start: {}, end: {}", span.start, span.end);
        }
    }
}
