// Test to verify span information is available in parser errors
// Uses the token_parser directly to check error spans without Salsa overhead.

use ast::token_parser;
use chumsky::Parser;
use chumsky::input::Stream;
use lexer::Lexer;

fn main() {
    let source = "x + "; // Incomplete expression

    let lexer = Lexer::new(source);
    let token_stream = Stream::from_iter(lexer.map(|(t, _s)| t));
    let (output, errors) = token_parser().parse(token_stream).into_output_errors();

    if errors.is_empty() {
        println!("Unexpected success: {:?}", output);
    } else {
        println!("Got errors as expected:");
        for error in &errors {
            println!("    Error: {:?}", error);
            let span_ref = error.span();
            println!(
                "      Span start: {}, end: {}",
                span_ref.start, span_ref.end
            );
        }
    }
}
