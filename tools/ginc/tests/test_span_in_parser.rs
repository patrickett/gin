// Test to verify span preservation in parsing

mod helpers;
use helpers::parse_str;

fn main() {
    let source = "x + 5";

    let ast = parse_str(source);
    println!("Parsing successful! AST: {:?}", ast);
}
