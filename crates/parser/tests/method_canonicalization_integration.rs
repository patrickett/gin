#![cfg(test)]

use internment::Intern;

#[test]
fn parse_from_str_canonicalizes_method_keys() {
    let source = "Factory has\n    new(count Int) Factory: return Factory\n    new(count Int, alignment Int) Factory: return Factory\n    size() Factory: return Factory\n";

    let ast = parser::parse_from_str(source);

    let name = Intern::from_ref("Factory.new");
    let members = &ast.defs.groups()[&name];
    assert_eq!(members.len(), 2);
    assert_ne!(members[0], members[1]);
    assert_eq!(ast.defs[&members[0]].params.as_ref().unwrap().len(), 1);
    assert_eq!(ast.defs[&members[1]].params.as_ref().unwrap().len(), 2);
    assert!(ast.defs.contains_key(&Intern::from_ref("Factory.size")));

    assert_eq!(ast.method_binds.len(), 3);
    assert_eq!(
        ast.defs.len(),
        3,
        "canonical defs should preserve method entries while dropping raw method keys"
    );
}
