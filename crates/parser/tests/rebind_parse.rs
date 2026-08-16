use ast::{BindOperator, BindValue, Expr};
use parser::query::SourceParseExt;

fn local_operators(source: &str) -> Vec<BindOperator> {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let main = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("main"))
        .expect("main");
    let BindValue::Body { exprs, .. } = &main.value else {
        panic!("expected main body");
    };
    exprs
        .iter()
        .filter_map(|expression| match &expression.value {
            Expr::Bind(bind) => Some(bind.operator.clone()),
            _ => None,
        })
        .collect()
}

fn body_expressions(source: &str) -> Vec<Expr> {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let main = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("main"))
        .expect("main");
    let BindValue::Body { exprs, .. } = &main.value else {
        panic!("expected main body");
    };
    exprs
        .iter()
        .map(|expression| expression.value.clone())
        .collect()
}

#[test]
fn fresh_mutable_bind_has_its_own_operator() {
    assert_eq!(
        local_operators("main:\n    value: 1\n    return 0"),
        [BindOperator::FreshMutable]
    );
}

#[test]
fn fresh_immutable_bind_has_its_own_operator() {
    assert_eq!(
        local_operators("main:\n    value := 1\n    return 0"),
        [BindOperator::FreshImmutable]
    );
}

#[test]
fn explicit_rebind_has_its_own_operator() {
    assert_eq!(
        local_operators("main:\n    value: 1\n    value:: 2\n    return 0"),
        [BindOperator::FreshMutable, BindOperator::Rebind]
    );
}

#[test]
fn typed_declaration_without_initializer_is_unassigned() {
    let output = "value Int\n".parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let value = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("value"))
        .expect("value");
    assert!(matches!(value.value, BindValue::Unassigned));
}

#[test]
fn record_projection_rebind_parses_as_record_set() {
    let expressions = body_expressions("main:\n    record.field:: 1\n    return 0");
    assert!(matches!(expressions[0], Expr::RecordSet { .. }));
}

#[test]
fn tuple_projection_rebind_parses_as_tuple_set() {
    let expressions = body_expressions("main:\n    tuple.0:: 1\n    return 0");
    assert!(matches!(expressions[0], Expr::TupleSet { .. }));
}

#[test]
fn indexed_projection_rebind_parses_as_buffer_set() {
    let expressions = body_expressions("main:\n    items.(index):: 1\n    return 0");
    assert!(matches!(expressions[0], Expr::BufSet { .. }));
}

#[test]
fn every_compound_rebind_maps_to_its_ordinary_operator() {
    let source = "main:\n    value+: 1\n    value-: 1\n    value*: 1\n    value/: 1\n    value%: 1\n    value&: 1\n    value|: 1\n    value^: 1\n    value<<: 1\n    value>>: 1\n    return 0";
    assert_eq!(
        local_operators(source),
        [
            BindOperator::Compound(ast::BinOp::Add),
            BindOperator::Compound(ast::BinOp::Subtract),
            BindOperator::Compound(ast::BinOp::Multiply),
            BindOperator::Compound(ast::BinOp::Divide),
            BindOperator::Compound(ast::BinOp::Modulo),
            BindOperator::Compound(ast::BinOp::BitAnd),
            BindOperator::Compound(ast::BinOp::BitOr),
            BindOperator::Compound(ast::BinOp::BitXor),
            BindOperator::Compound(ast::BinOp::ShiftLeft),
            BindOperator::Compound(ast::BinOp::ShiftRight),
        ]
    );
}

#[test]
fn projected_compound_rebind_retains_the_operator() {
    let output =
        "main:\n    record.field+: 1\n    tuple.0-: 2\n    items.(index)*: 3\n    return 0"
            .parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    let main = output
        .ast
        .defs
        .get(&internment::Intern::from_ref("main"))
        .expect("main");
    let BindValue::Body { exprs, .. } = &main.value else {
        panic!("expected main body");
    };

    assert!(matches!(
        exprs[0].value,
        Expr::RecordSet {
            operator: Some(ast::BinOp::Add),
            ..
        }
    ));
    assert!(matches!(
        exprs[1].value,
        Expr::TupleSet {
            operator: Some(ast::BinOp::Subtract),
            ..
        }
    ));
    assert!(matches!(
        exprs[2].value,
        Expr::BufSet {
            operator: Some(ast::BinOp::Multiply),
            ..
        }
    ));
}
