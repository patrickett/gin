#![cfg(test)]

use internment::Intern;

#[test]
fn parse_from_str_canonicalizes_method_keys() {
    let source = "\
Factory has
    new(count Int) Factory: Factory
    new(count Int, alignment Int) Factory: Factory
    size() Factory: Factory
";

    let ast = parser::parse_from_str(source);

    let overload_arity_1 = Intern::<String>::new("Factory.new$arity1".to_string());
    let overload_arity_2 = Intern::<String>::new("Factory.new$arity2".to_string());
    let non_overloaded = Intern::<String>::new("Factory.size".to_string());
    let pre_canonical_overload = Intern::<String>::new("Factory.new".to_string());

    assert!(ast.defs.contains_key(&overload_arity_1));
    assert!(ast.defs.contains_key(&overload_arity_2));
    assert!(ast.defs.contains_key(&non_overloaded));
    assert!(!ast.defs.contains_key(&pre_canonical_overload));

    assert_eq!(ast.method_binds.len(), 3);
    assert_eq!(
        ast.defs.len(),
        3,
        "canonical defs should preserve method entries while dropping raw method keys"
    );
}
