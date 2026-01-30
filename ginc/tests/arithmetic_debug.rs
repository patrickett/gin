use ginc::frontend::parser::Parsable;

#[test]
fn test_parse_arithmetic_debug() {
    let src = "add(a, b): a + b\n";

    println!("Source: {}", src);
    println!("Tokens:");
    use ginc::frontend::lexer::GinLexer;
    for (i, (tok, span)) in GinLexer::new(src).enumerate() {
        println!("  {}: {:?} at {:?}", i, tok, span);
    }

    let ast = src.to_ast().unwrap();
    println!("AST nodes: {}", ast.nodes.len());

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert!(node.imports.is_empty());
        assert_eq!(node.defs.len(), 1);
        assert_eq!(node.tags.len(), 0);
    }
}
