use ast::{BindValue, ConstValue, Expr, Literal, Typed};
use internment::Intern;
use parser::query::SourceParseExt;

fn visit_typed(expr: &Typed<Expr>, visit: &mut impl FnMut(&Typed<Expr>)) {
    visit(expr);
    let _ = ast::folder::walk_typed_expr_children(&expr.value, &mut |child| {
        visit_typed(child, visit);
        std::ops::ControlFlow::Continue(())
    });
}

fn visit_value(value: &BindValue, visit: &mut impl FnMut(&Typed<Expr>)) {
    match value {
        BindValue::Expr(expr) => visit_typed(expr, visit),
        BindValue::Body { exprs, ret } => {
            for expr in exprs.iter().chain(&ret.values) {
                visit_typed(expr, visit);
            }
        }
        BindValue::Unassigned => {}
    }
}

#[test]
fn static_member_materialization_covers_expression_children_and_preserves_metadata() {
    for (body, expected) in [
        ("sample: Settings.value\n", 1),
        ("sample: -Settings.value\n", 1),
        ("sample: consume(Settings.value)\n", 1),
        ("sample: Settings.value + Settings.value\n", 2),
        ("sample: [Settings.value]\n", 1),
        ("sample: [Settings.value; 2]\n", 1),
        ("sample: [Settings.value, Settings.value]\n", 2),
        ("sample: [value: Settings.value]\n", 1),
        (
            "sample:\n    local:\n        Settings.value\n        Settings.value\n    return local\n",
            2,
        ),
        (
            "sample:\n    while 0 is 1 do\n        Settings.value\n    return\n",
            1,
        ),
        (
            "sample:\n    while Settings.value is 15 do\n        0\n    return\n",
            1,
        ),
    ] {
        let source = format!(
            "Count is in 0 to 255\nDefaults has value Count\nSettings has Defaults has value: 15\n{body}"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{body}: {:?}", parsed.symptoms);
        let mut ast = parsed.ast;
        let sample = ast::DefinitionId::named(Intern::from_ref("sample"));
        let mut before = Vec::new();
        visit_value(&ast.defs[&sample].value, &mut |expr| {
            if matches!(&expr.value, Expr::FnCall(call) if call.path.value.to_qualified_string() == "Settings.value")
            {
                before.push((expr.span_id, expr.ty.clone()));
            }
        });
        assert_eq!(before.len(), expected, "{body}");
        typecheck::materialize_type_static_access(&mut ast);
        let mut after = Vec::new();
        visit_value(&ast.defs[&sample].value, &mut |expr| {
            assert!(
                !matches!(&expr.value, Expr::FnCall(call) if call.path.value.to_qualified_string() == "Settings.value"),
                "unmaterialized access: {body}"
            );
            if matches!(&expr.value, Expr::Lit(Literal::Int(value)) if *value == 15.into()) {
                assert_eq!(expr.const_value, Some(Box::new(ConstValue::Int(15.into()))));
                after.push((expr.span_id, expr.ty.clone()));
            }
        });
        assert_eq!(after, before, "{body}");
    }
}

#[test]
fn runtime_or_unknown_members_are_not_replaced_by_static_defaults() {
    for access in ["receiver.value", "Settings.missing", "Unknown.value"] {
        let source = format!("Settings has Defaults has value: 15\nsample: {access}\n");
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut ast = parsed.ast;
        let sample = ast::DefinitionId::named(Intern::from_ref("sample"));
        let before = ast.defs[&sample].value.clone();
        typecheck::materialize_type_static_access(&mut ast);
        assert_eq!(ast.defs[&sample].value, before, "{access}");
    }
}
