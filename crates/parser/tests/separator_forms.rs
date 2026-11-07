//! Commas and newlines separate list elements, parameters, and `has` members alike.
use ast::{Expr, Item, TypeBody};
use parser::parse_strict;

fn counts(source: &str) -> usize {
    let file = parse_strict(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    match &file.items[0].value {
        Item::Binding(binding) => match (&binding.parameters, &binding.value) {
            (Some(parameters), _) => parameters.value.len(),
            (None, Some(value)) => match &value.expression.value {
                Expr::Array(array) => array.elements.len(),
                other => panic!("{source:?}: not an array: {other:?}"),
            },
            _ => panic!("{source:?}: neither parameters nor value"),
        },
        Item::Type(declaration) => match &declaration.body.value {
            TypeBody::Has(members) => members.len(),
            other => panic!("{source:?}: not a has body: {other:?}"),
        },
        other => panic!("{source:?}: unexpected item {other:?}"),
    }
}

fn same(expected: usize, forms: &[&str]) {
    for source in forms {
        assert_eq!(counts(source), expected, "{source:?}");
    }
}

#[test]
fn list_literals() {
    same(
        3,
        &["items: [1, 2, 3]\n", "items: [\n    1\n    2\n    3\n]\n"],
    );
}

#[test]
fn parameter_lists() {
    same(
        4,
        &[
            "create_user(first, last, age, perms) User: 1\n",
            "create_user(\n    first\n    last\n    age\n    perms\n) User: 1\n",
        ],
    );
}

#[test]
fn has_members() {
    same(
        2,
        &[
            "User has first Int, last Int\n",
            "User has\n    first Int\n    last Int\n",
        ],
    );
}
