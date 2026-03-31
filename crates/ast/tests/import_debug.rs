mod helpers;
use helpers::parse_str;

#[test]
fn test_parse_import_debug() {
    let src = "use http.web as h";

    println!("=== Testing import parsing ===");
    println!("Source: '{}'", src);

    use lexer::GinLexer;
    let tokens: Vec<_> = GinLexer::new(src).collect();
    println!("Tokens from lexer:");
    for (i, (token, span)) in tokens.iter().enumerate() {
        println!("  [{}] {:?} at {:?}", i, token, span);
    }

    let ast = parse_str(src);

    assert_eq!(ast.uses().len(), 1);
    assert_eq!(ast.defs().len(), 0);
    assert_eq!(ast.tags().len(), 0);
}
