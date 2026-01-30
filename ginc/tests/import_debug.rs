use ginc::frontend::parser::Parsable;

#[test]
fn test_parse_import_debug() {
    let src = "use http.web as h";

    println!("=== Testing import parsing ===");
    println!("Source: '{}'", src);

    // Let's manually check what tokens we get
    use ginc::frontend::lexer::GinLexer;
    let tokens: Vec<_> = GinLexer::new(src).collect();
    println!("Tokens from lexer:");
    for (i, (token, span)) in tokens.iter().enumerate() {
        println!("  [{}] {:?} at {:?}", i, token, span);
    }

    let ast = src.to_ast().unwrap();
    println!("AST nodes: {}", ast.nodes.len());

    // one node
    assert_eq!(ast.nodes.len(), 1);

    for (_path, node) in ast.nodes {
        assert_eq!(node.imports.len(), 1);
        assert_eq!(node.defs.len(), 0);
        assert_eq!(node.tags.len(), 0);
    }
}
