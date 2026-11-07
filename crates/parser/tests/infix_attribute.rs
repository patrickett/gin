//! `#infix('op')` marks a method of a `has` block; it needs an operator string.
use ast::{FieldAttribute, Item, TypeBody, TypeMember};

fn method<'a>(file: &'a ast::SourceFile, member: usize) -> &'a ast::Binding {
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("type")
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("members")
    };
    let TypeMember::Binding(binding) = &members[member].value else {
        panic!("method")
    };
    binding
}

const MONEY: &str =
    "Money has\n  cents in 0 to 9\n  #infix('+')\n  add(self, rhs Self) Self: self\n";

#[test]
fn infix_precedes_a_method_and_records_its_operator_string() {
    let file = parser::parse_strict(MONEY).unwrap();
    let add = method(&file, 1);
    assert_eq!(add.name.value, "add");
    let [attribute] = &add.attributes[..] else {
        panic!("one attribute")
    };
    let FieldAttribute::Infix { operator, .. } = attribute.value else {
        panic!("infix")
    };
    assert_eq!(file.span_table.get(operator).extract(MONEY), "'+'");
    assert_eq!(
        file.span_table.get(attribute.span_id).extract(MONEY),
        "#infix('+')"
    );
}

#[test]
fn infix_works_with_an_indented_method_body() {
    let source = "Money has\n  cents in 0 to 9\n  #infix('+')\n  add(self, rhs Self) Self:\n    return self\n";
    let file = parser::parse_strict(source).unwrap();
    let add = method(&file, 1);
    assert_eq!(add.attributes.len(), 1);
    assert!(add.value.is_some());
    let span = file.span_table.get(file.items[0].span_id);
    assert_eq!(span.extract(source), source.trim_end());
}

#[test]
fn infix_attributes_compose_with_commas() {
    let source = "Money has\n  #infix('+'), #infix('-')\n  add(self, rhs Self) Self: self\n";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(method(&file, 0).attributes.len(), 2);
}

#[test]
fn infix_without_an_operator_or_on_a_field_is_a_syntax_error() {
    for source in [
        "Money has\n  #infix\n  add(self, rhs Self) Self: self\n",
        "Money has\n  #infix()\n  add(self, rhs Self) Self: self\n",
        "Money has\n  #infix(+)\n  add(self, rhs Self) Self: self\n",
        "Money has\n  #infix('+')\n  cents in 0 to 9\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}

/// `#private` marks a method as well as a field, so it may sit with `#infix`.
#[test]
fn private_and_infix_may_mark_one_method() {
    let source = "Money has\n  #private\n  #infix('+')\n  add(self, rhs Self) Self: self\n";
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(method(&file, 0).attributes.len(), 2);
}
