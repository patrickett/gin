mod helpers;
use helpers::parse_str;

#[test]
fn identical_sources_are_equal() {
    let src = "f(x): x\n";
    let ast1 = parse_str(src);
    let ast2 = parse_str(src);
    assert_eq!(ast1, ast2);
}

#[test]
fn identical_sources_have_same_hash() {
    use std::hash::{Hash, Hasher};
    let src = "f(x): x\n";
    let ast1 = parse_str(src);
    let ast2 = parse_str(src);

    let hash_of = |ast: &ast::FileAst| -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ast.hash(&mut h);
        h.finish()
    };

    assert_eq!(hash_of(&ast1), hash_of(&ast2));
}

#[test]
fn different_sources_are_not_equal() {
    let ast1 = parse_str("f(x): x\n");
    let ast2 = parse_str("g(x): x\n");
    assert_ne!(ast1, ast2);
}

#[test]
fn content_hash_differs_for_structurally_different_asts() {
    let h1 = parse_str("f(x): x\n").compute_content_hash();
    let h2 = parse_str("g(x): x\n").compute_content_hash();
    assert_ne!(h1, h2);
}
