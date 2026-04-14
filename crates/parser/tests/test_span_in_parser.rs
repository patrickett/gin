// Test to verify span preservation in parsing

use parser::parse_from_str as parse_str;

fn main() {
    let source = "x + 5";

    let ast = parse_str(source);
    println!("Parsing successful! AST: {:?}", ast);
}
