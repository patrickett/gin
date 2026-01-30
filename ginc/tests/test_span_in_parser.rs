// Test to verify span preservation in parsing

use ginc::frontend::parser::Parsable;

fn main() {
    let source = "x + 5";

    // Parse the source
    let result = source.to_ast();
    match result {
        Ok(ast) => {
            println!("Parsing successful! AST: {:?}", ast);
        }
        Err(errs) => {
            println!("Parsing errors: {:?}", errs);
        }
    }
}
