//! `place :: value`: reassignment of a local or a field path.
use ast::{Expr, Item, SourceFile, Statement, SyntaxNodeKind};

fn statements(file: &SourceFile) -> &[ast::Spanned<Statement>] {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block body")
    };
    &block.statements
}

#[test]
fn a_local_can_be_reassigned() {
    let source = "f(x mut):\n  x :: 3\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Assign(assign) = &statements(&file)[0].value else {
        panic!("assignment")
    };
    assert!(matches!(&assign.place.value, Expr::Name(name) if name == "x"));
    assert_eq!(file.span_table.get(assign.operator).extract(source), "::");
    assert_eq!(
        file.span_table.get(assign.value.span_id).extract(source),
        "3"
    );
}

#[test]
fn a_field_path_can_be_reassigned() {
    let source = "f(o mut):\n  o.inner.count :: o.inner.count + 1\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Assign(assign) = &statements(&file)[0].value else {
        panic!("assignment")
    };
    let Expr::Member { base, name } = &assign.place.value else {
        panic!("member place")
    };
    assert_eq!(name.value, "count");
    assert!(matches!(&base.value, Expr::Member { .. }));
    assert_eq!(
        file.span_table.get(assign.place.span_id).extract(source),
        "o.inner.count"
    );
    assert!(matches!(assign.value.value, Expr::Binary { .. }));
}

#[test]
fn the_value_is_an_ordinary_expression_including_a_conditional() {
    let source = "f(x mut, y):\n  x :: when y < 3 then 1 else 2\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Assign(assign) = &statements(&file)[0].value else {
        panic!("assignment")
    };
    assert!(matches!(assign.value.value, Expr::Conditional(_)));
}

#[test]
fn assignment_works_beside_other_statements() {
    let source = "f(x mut):\n  y: 1\n  x :: y\n  z := 2\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let kinds: Vec<_> = statements(&file)
        .iter()
        .map(|statement| matches!(statement.value, Statement::Assign(_)))
        .collect();
    assert_eq!(kinds, [false, true, false, false]);
}

#[test]
fn the_query_exposes_the_assignment_and_its_operator() {
    let source = "f(x mut):\n  x :: 3\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let hit = file.node_at_offset(source.find("::").unwrap()).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::ReassignOperator));
}

#[test]
fn malformed_assignments_are_syntax_errors() {
    for source in [
        "f(x mut):\n  x ::\n  return\n",
        "f(x mut):\n  :: 3\n  return\n",
        "f(x mut):\n  x. :: 3\n  return\n",
        "f(x mut):\n  3 :: 3\n  return\n",
        "f(x mut):\n  x :: 3 :: 4\n  return\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}

#[test]
fn a_compound_assignment_names_its_operator_before_the_colon() {
    let source = "f(x mut):\n  x +: 3\n  x.y <<: 1\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let statements = statements(&file);
    let Statement::Assign(add) = &statements[0].value else {
        panic!("assignment")
    };
    let compound = add.compound.as_ref().expect("compound operator");
    assert_eq!(compound.value, ast::BinaryOperator::Add);
    assert_eq!(file.span_table.get(compound.span_id).extract(source), "+");
    assert_eq!(file.span_table.get(add.operator).extract(source), ":");
    let Statement::Assign(shift) = &statements[1].value else {
        panic!("assignment")
    };
    assert_eq!(
        shift.compound.as_ref().map(|c| c.value),
        Some(ast::BinaryOperator::ShiftLeft)
    );
    assert!(matches!(&shift.place.value, Expr::Member { .. }));
}

#[test]
fn a_plain_reassignment_has_no_compound_operator() {
    let source = "f(x mut):\n  x :: 3\n  return\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Assign(assign) = &statements(&file)[0].value else {
        panic!("assignment")
    };
    assert!(assign.compound.is_none());
}
