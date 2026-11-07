//! An assignment target is a local followed by any mix of `.field` and `.[index]`.
use ast::{Expr, Item, SourceFile, Statement};

fn first_assignment(file: &SourceFile) -> &ast::AssignStatement {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("binding")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block")
    };
    let Statement::Assign(assign) = &block.statements[0].value else {
        panic!("assignment: {:?}", block.statements[0].value)
    };
    assign
}

fn place(source: &str) -> ast::Expression {
    let file = parser::parse_strict(source).unwrap_or_else(|e| panic!("{source:?}: {e:?}"));
    first_assignment(&file).place.clone()
}

#[test]
fn an_element_is_a_place() {
    let place = place("main:\n    buffer.[index] :: byte\n    return 0\n");
    let Expr::Select(select) = &place.value else {
        panic!("select: {place:?}")
    };
    assert!(matches!(&select.base.value, Expr::Name(name) if name == "buffer"));
    assert!(matches!(&select.index.value, Expr::Name(name) if name == "index"));
    assert!(select.length.is_none());
}

#[test]
fn a_field_of_an_element_is_a_place() {
    let place = place("main:\n    pointer.[0].field :: 1\n    return 0\n");
    let Expr::Member { base, name } = &place.value else {
        panic!("member: {place:?}")
    };
    assert_eq!(name.value, "field");
    assert!(matches!(base.value, Expr::Select(_)));
}

#[test]
fn an_element_of_a_field_and_a_nested_index() {
    for source in [
        "main:\n    a.b.[i] :: 1\n    return 0\n",
        "main:\n    a.[i].[j] :: 1\n    return 0\n",
        "main:\n    a.[i + 1].b.[2] :: 1\n    return 0\n",
    ] {
        place(source);
    }
}

#[test]
fn a_compound_assignment_to_an_element() {
    let file = parser::parse_strict("main:\n    counts.[i] +: 1\n    return 0\n").unwrap();
    let assign = first_assignment(&file);
    assert!(assign.compound.is_some());
    assert!(matches!(assign.place.value, Expr::Select(_)));
}

#[test]
fn an_unclosed_index_is_an_error() {
    assert!(parser::parse_strict("main:\n    a.[i :: 1\n    return 0\n").is_err());
}
