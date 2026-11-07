//! `Type has TraitA and TraitB` names provided traits; members follow on indented lines.
use ast::{Item, TypeBody, TypeMember};

fn members(file: &ast::SourceFile, item: usize) -> &[ast::Spanned<TypeMember>] {
    let Item::Type(declaration) = &file.items[item].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("has")
    };
    members
}

#[test]
fn provisions_precede_the_members_in_written_order() {
    let source = "Byte has Numeric and Eq\n    close(self): return\n";
    let file = parser::parse_strict(source).unwrap();
    let members = members(&file, 0);
    let TypeMember::Provision(first) = &members[0].value else {
        panic!("first provision")
    };
    let TypeMember::Provision(second) = &members[1].value else {
        panic!("second provision")
    };
    assert_eq!(
        (first.value.as_str(), second.value.as_str()),
        ("Numeric", "Eq")
    );
    assert_eq!(
        file.span_table.get(members[0].span_id).extract(source),
        "Numeric"
    );
    let TypeMember::Binding(close) = &members[2].value else {
        panic!("method")
    };
    assert_eq!(close.name.value, "close");
}

#[test]
fn a_single_provision_may_stand_alone() {
    let file = parser::parse_strict("Byte has Numeric\n").unwrap();
    assert!(matches!(members(&file, 0), [m] if matches!(m.value, TypeMember::Provision(_))));
}

#[test]
fn a_trait_is_a_has_block_of_method_signatures() {
    let source = "Numeric has\n    #infix('+')\n    add(self, rhs Self)\n";
    let file = parser::parse_strict(source).unwrap();
    let TypeMember::Binding(add) = &members(&file, 0)[0].value else {
        panic!("signature")
    };
    assert!(add.value.is_none());
    assert_eq!(add.attributes.len(), 1);
}

#[test]
fn provisions_do_not_mix_with_inline_members_or_dangling_and() {
    for source in [
        "Byte has Numeric close(self): return\n",
        "Byte has Numeric and\n",
        "Byte has Numeric and 3\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}
