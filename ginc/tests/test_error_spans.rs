// Test to verify span information is available in parser errors
use ginc::frontend::parser::Parsable;

fn main() {
    // Test with invalid syntax to trigger an error
    let source = "x + "; // Incomplete expression

    let result = source.to_ast();
    match result {
        Ok(ast) => {
            println!("Unexpected success: {:?}", ast);
        }
        Err(errs) => {
            println!("Got errors as expected:");
            for (error_list, path) in errs {
                println!("  Path: {:?}", path);
                for error in error_list {
                    println!("    Error: {:?}", error);
                    // Check if the error has span information
                    let span_ref = error.span();
                    println!(
                        "      Span start: {}, end: {}",
                        span_ref.start, span_ref.end
                    );
                }
            }
        }
    }
}
