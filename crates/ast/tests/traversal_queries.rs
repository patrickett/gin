use ast::{BindValue, Expr};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn parsed_main_expr(source: &str) -> (ast::FileAst, Expr) {
    let ast = TokenCursor::parse_source(source);
    let bind = ast
        .defs
        .get(&Intern::from_ref("main"))
        .expect("main definition");
    let BindValue::Expr(expr) = &bind.value else {
        panic!("expression body");
    };
    (ast.clone(), expr.value.clone())
}

#[test]
fn parse_direct_children_follow_source_order() {
    let cases = [
        ("main: call(left, right)\n", "call", 2),
        ("main: left + right\n", "binary", 2),
        ("main: (left, right)\n", "call", 0),
        ("main: (x: left, y: right)\n", "record", 2),
        ("main: [left, right]\n", "sequence literal", 2),
        ("main: Shape(left, right)\n", "tag call", 2),
        ("main: left to right\n", "range", 2),
        ("main: (init; size)\n", "fixed-array allocation", 2),
        ("main: buffer.(index)\n", "buffer get", 2),
        ("main: buffer.(index):: value\n", "buffer set", 3),
        ("main: tuple.0\n", "value row get", 1),
        ("main: tuple.0:: value\n", "value row set", 2),
        ("main: record.field\n", "record get", 1),
        ("main: record.field:: value\n", "record set", 2),
        ("main: value as Int\n", "cast", 1),
        ("main: @value\n", "take pointer", 1),
        ("main: *value\n", "dereference", 1),
        ("main: ref value\n", "reference", 1),
        ("main: mut value\n", "mutable reference", 1),
        ("main: deref value\n", "dereference", 1),
        ("main: -value\n", "negate", 1),
        ("main: dispatch(spec, left, right)\n", "call", 3),
        ("main: \"value: \\(needle)\"\n", "format string", 1),
        (
            "main: when condition is True then body else fallback\n",
            "when",
            3,
        ),
        (
            "main: when subject is True then body else fallback\n",
            "when",
            3,
        ),
        (
            "main: when subject is True then body else fallback\n",
            "when",
            3,
        ),
        ("main: while condition is True\n    body\nloop\n", "loop", 2),
        ("main: for item in iterable\n    body\nloop\n", "loop", 3),
        (
            "main: if condition is True\n    body\nreturn fallback\n",
            "if",
            3,
        ),
    ];

    for (source, expected_variant, expected_count) in cases {
        let (_, expr) = parsed_main_expr(source);
        let actual_variant = match &expr {
            Expr::FnCall(_) => "call",
            Expr::Binary(_) => "binary",
            Expr::SequenceLiteral(_) => "sequence literal",
            Expr::ArrayLit(_) => "array",
            Expr::RecordLit(_) => "record",
            Expr::List(_) => "list",
            Expr::TagCall(_) => "tag call",
            Expr::Range(_) => "range",
            Expr::FixedArrayAlloc { .. } => "fixed-array allocation",
            Expr::BufGet { .. } => "buffer get",
            Expr::BufSet { .. } => "buffer set",
            Expr::AggregateGet { .. } => "value row get",
            Expr::AggregateSet { .. } => "value row set",
            Expr::RecordGet { .. } => "record get",
            Expr::RecordSet { .. } => "record set",
            Expr::Cast { .. } => "cast",
            Expr::TakePtr(_) => "take pointer",
            Expr::Deref(_) => "dereference",
            Expr::Ref { mutable: false, .. } => "reference",
            Expr::Ref { mutable: true, .. } => "mutable reference",
            Expr::Negate(_) => "negate",
            Expr::FormatString(_) => "format string",
            Expr::When(_) => "when",
            Expr::Loop(_) => "loop",
            Expr::If(_) => "if",
            other => panic!("unexpected expression for {source}: {other:?}"),
        };
        let mut children = Vec::new();
        let _ = ast::folder::walk_expr_children(&expr, &mut |span_id, _| {
            children.push(span_id);
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(actual_variant, expected_variant, "{source}");
        assert_eq!(children.len(), expected_count, "{source}");
    }
}

#[test]
fn parse_direct_children_support_early_exit() {
    let (_, expr) = parsed_main_expr("main: call(first, second)\n");
    let mut visits = 0;

    let result = ast::folder::walk_expr_children(&expr, &mut |_, _| {
        visits += 1;
        std::ops::ControlFlow::Break(())
    });

    assert!(result.is_break());
    assert_eq!(visits, 1);
}

#[test]
fn mutable_parse_traversal_visits_all_allocation_children() {
    let (_, mut expr) = parsed_main_expr("main: (initializer; size)\n");
    let mut visits = 0;

    let _ = ast::folder::walk_expr_children_mut(&mut expr, &mut |_, _| {
        visits += 1;
        std::ops::ControlFlow::Continue(())
    });

    assert_eq!(visits, 2);
}

#[test]
fn typed_direct_children_cover_lowered_multi_child_shapes() {
    let cases = [
        ("main: call(left, right)\n", "call", 2),
        ("main: left + right\n", "intrinsic", 2),
        ("main: (left, right)\n", "call", 0),
        ("main: (x: left, y: right)\n", "record", 2),
        ("main: [left, right]\n", "array", 2),
        ("main: Shape(left, right)\n", "tag call", 2),
        ("main: left to right\n", "range", 2),
        ("main: (init; 3)\n", "fixed-array allocation", 1),
        ("main: buffer.(index)\n", "buffer get", 2),
        ("main: buffer.(index):: value\n", "buffer set", 3),
        ("main: tuple.0\n", "value row get", 1),
        ("main: tuple.0:: value\n", "value row set", 2),
        ("main: record.field:: value\n", "record set", 2),
        ("main: value as Int\n", "cast", 1),
        ("main: @value\n", "take pointer", 1),
        ("main: *value\n", "dereference", 1),
        ("main: ref value\n", "reference", 1),
        ("main: deref value\n", "dereference", 1),
        ("main: -value\n", "negate", 1),
        ("main: dispatch(spec, left, right)\n", "call", 3),
        ("main: \"value: \\(needle)\"\n", "invalid", 0),
        (
            "main: when condition is True then body else fallback\n",
            "when",
            3,
        ),
        ("main: while condition is True\n    body\nloop\n", "loop", 2),
        ("main: for item in iterable\n    body\nloop\n", "loop", 2),
        (
            "main: if condition is True\n    body\nreturn fallback\n",
            "if",
            3,
        ),
    ];

    for (source, expected_variant, expected_count) in cases {
        let ast = TokenCursor::parse_source(source);
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        let bind = typed
            .defs
            .get(&DefId(Intern::from_ref("main")))
            .expect("main definition");
        let root = match bind.body {
            BindBody::Expr(id) => id,
            _ => panic!("expression body"),
        };
        let kind = &typed.exprs.kind[root.as_usize()];
        let actual_variant = match kind {
            TypedExprKind::FnCall { .. } => "call",
            TypedExprKind::IntrinsicCall { .. } => "intrinsic",
            TypedExprKind::Binary { .. } => "binary",
            TypedExprKind::InvalidOperator { .. } => "binary",
            TypedExprKind::RecordValue(_) => "record",
            TypedExprKind::ArrayLit(_) => "array",
            TypedExprKind::List(_) => "list",
            TypedExprKind::TagCall { .. } => "tag call",
            TypedExprKind::Range { .. } => "range",
            TypedExprKind::FixedArrayAlloc { .. } => "fixed-array allocation",
            TypedExprKind::BufGet { .. } => "buffer get",
            TypedExprKind::BufSet { .. } => "buffer set",
            TypedExprKind::AggregateGet { .. } => "value row get",
            TypedExprKind::AggregateSet { .. } => "value row set",
            TypedExprKind::RecordSet { .. } => "record set",
            TypedExprKind::Cast { .. } => "cast",
            TypedExprKind::TakePtr(_) => "take pointer",
            TypedExprKind::Deref(_) => "dereference",
            TypedExprKind::Ref(_) => "reference",
            TypedExprKind::Negate(_) => "negate",
            TypedExprKind::Invalid => "invalid",
            TypedExprKind::When(_) => "when",
            TypedExprKind::Loop(_) => "loop",
            TypedExprKind::If(_) => "if",
            other => panic!("unexpected typed expression for {source}: {other:?}"),
        };
        let mut children = Vec::new();
        let _ = typecheck::typed::walk_expr_children(kind, &mut |child| {
            children.push(child);
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(actual_variant, expected_variant, "{source}");
        assert_eq!(children.len(), expected_count, "{source}");
    }
}

#[test]
fn body_expression_children_cover_bind_reassign_and_destructure() {
    let source = "\
main:
    local Int
    local:: initializer
    Some(x: item) := source
return item
";
    let ast = TokenCursor::parse_source(source);
    let bind = ast
        .defs
        .get(&Intern::from_ref("main"))
        .expect("main definition");
    let BindValue::Body { exprs, .. } = &bind.value else {
        panic!("block body");
    };
    let parse_expected = [("bind", 0), ("bind", 1), ("destructure", 1)];
    for (expr, (expected_variant, expected_count)) in exprs.iter().zip(parse_expected) {
        let actual_variant = match &expr.value {
            Expr::Bind(_) => "bind",
            Expr::Destructure { .. } => "destructure",
            other => panic!("unexpected parse body expression: {other:?}"),
        };
        let mut children = 0;
        let _ = ast::folder::walk_expr_children(&expr.value, &mut |_, _| {
            children += 1;
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(actual_variant, expected_variant);
        assert_eq!(children, expected_count, "{expected_variant}");
    }

    let typed = transform(&ast, FileId(0), &TransformCtx::new());
    let bind = typed
        .defs
        .get(&DefId(Intern::from_ref("main")))
        .expect("typed main definition");
    let BindBody::Body { exprs, .. } = &bind.body else {
        panic!("typed block body");
    };
    let expected = [("bind", 0), ("reassign", 1), ("destructure", 1)];
    for (id, (expected_variant, expected_count)) in exprs.iter().zip(expected) {
        let kind = &typed.exprs.kind[id.as_usize()];
        let actual_variant = match kind {
            TypedExprKind::Bind { .. } => "bind",
            TypedExprKind::Reassign { .. } => "reassign",
            TypedExprKind::Destructure { .. } => "destructure",
            other => panic!("unexpected body expression: {other:?}"),
        };
        let mut children = 0;
        let _ = typecheck::typed::walk_expr_children(kind, &mut |_| {
            children += 1;
            std::ops::ControlFlow::Continue(())
        });
        assert_eq!(actual_variant, expected_variant);
        assert_eq!(children, expected_count, "{expected_variant}");
    }
}

#[test]
fn call_eat_argument_is_rejected() {
    let source = "main: call(eat value)\n";
    let ast = TokenCursor::parse_source(source);
    assert!(
        ast.parse_warnings
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "parse-removed-consume-arg-syntax"),
        "expected a parse warning for removed consume-argument syntax",
    );
}

#[test]
fn compile_time_validation_observes_nested_constructor_arguments() {
    let source = "\
Shape has value Type
runtime(value Type) Type: value
bad(ty Type) Type := Shape(runtime(ty))
";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );

    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| {
            flaw.code.slug() == "compile-time-runtime-call" && flaw.arg("bind_name") == Some("bad")
        }),
        "expected compile-time-runtime-call on `bad`, got: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn reference_lookup_observes_nested_call_and_binary_children() {
    let source = "needle: 1\nmain: outer(1 + inner(needle))\n";
    let ast = TokenCursor::parse_source(source);

    let references = ast.find_references("needle");

    assert_eq!(
        references,
        vec![source.rfind("needle").unwrap()..source.len() - 3]
    );
}

#[test]
fn cursor_lookup_returns_innermost_nested_expression() {
    let source = "needle: 1\nmain: outer(1 + inner(needle))\n";
    let ast = TokenCursor::parse_source(source);
    let byte_pos = source.rfind("needle").unwrap();

    let (expr, span_id) = ast.expr_at_byte(byte_pos).expect("nested expression");

    assert!(matches!(
        expr,
        Expr::FnCall(call)
            if call.path.root.as_str() == "needle" && call.path.segments.is_empty()
    ));
    assert!(ast.span_table.contains(span_id, byte_pos));
}
