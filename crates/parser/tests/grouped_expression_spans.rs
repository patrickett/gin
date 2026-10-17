use ast::BindValue;
use parser::query::SourceParseExt;

#[test]
fn grouped_values_include_their_outer_parentheses_in_source_spans() {
    for value in [
        "(1 + 2)",
        "((1 + 2) * 3)",
        "(when flag is True then 1 else 2)",
    ] {
        for source in [
            format!("value: {value}\n"),
            format!("value:\n    return {value}\n"),
        ] {
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            let bind = parsed.ast.defs.values().next().unwrap();
            let expr = match &bind.value {
                BindValue::Expr(expr) => expr.as_ref(),
                BindValue::Body { ret, .. } => &ret.values[0],
                BindValue::Unassigned => panic!("expected a value"),
            };
            assert_eq!(
                parsed.ast.span_table.get(expr.span_id).extract(&source),
                value
            );
        }
    }
}
