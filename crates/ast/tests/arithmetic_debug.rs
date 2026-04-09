use ast::parse_from_str as parse_str;

#[test]
fn test_parse_arithmetic_debug() {
    let src = "add(a, b): a + b\n";

    println!("Source: {}", src);
    println!("Tokens:");
    use lexer::Lexer;
    for (i, (tok, span)) in Lexer::new(src).enumerate() {
        println!("  {}: {:?} at {:?}", i, tok, span);
    }

    let ast = parse_str(src);

    assert!(ast.uses().is_empty());
    assert_eq!(ast.defs().len(), 1);
    assert_eq!(ast.tags().len(), 0);
}
