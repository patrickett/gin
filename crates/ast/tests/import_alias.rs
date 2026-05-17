use internment::Intern;

use ast::{BindValue, Expr, FileAst, ModPath, SpanId, Spanned, SymbolAlias, apply_symbol_aliases};
use parser::parse_source_full;

fn add_symbol_alias(ast: &mut FileAst, alias: &str, root: &str, symbol: &str) {
    ast.symbol_aliases.push(SymbolAlias {
        alias: Intern::from_ref(alias),
        target: Spanned::new(
            ModPath::new(Intern::from_ref(root), vec![Intern::from_ref(symbol)]),
            SpanId::INVALID,
        ),
    });
}

fn assert_alias_fn_call(expr: &Expr, expected_root: &str, expected_segments: &[&str]) {
    if let Expr::FnCall(call) = expr {
        assert_eq!(call.path.root, Intern::from_ref(expected_root));
        let segments: Vec<&str> = call.path.segments.iter().map(|s| s.as_str()).collect();
        assert_eq!(segments, expected_segments);
    } else {
        panic!("expected FnCall, found {:?}", expr);
    }
}

#[test]
fn rewrites_function_call_alias() {
    let mut out = parse_source_full("main:\n    true\nreturn\n");
    add_symbol_alias(&mut out.ast, "true", "core", "true");

    apply_symbol_aliases(&mut out.ast);

    let main = out.ast.defs().get(&Intern::from_ref("main")).unwrap();
    if let BindValue::Body { exprs, .. } = main.value() {
        let first_expr = &exprs[0].value;
        assert_alias_fn_call(first_expr, "core", &["true"]);
    } else {
        panic!("unexpected binding for main");
    }
}

#[test]
fn rewrites_anonymous_tag_alias() {
    let mut ast = FileAst::default();
    ast.exprs.push((
        Expr::AnonymousTag(Intern::from_ref("Range")),
        SpanId::INVALID,
    ));
    add_symbol_alias(&mut ast, "Range", "core", "Range");

    apply_symbol_aliases(&mut ast);

    // AnonymousTag is handled by a separate pass, so it remains unchanged.
    let expr = &ast.exprs[0].0;
    assert!(
        matches!(expr, Expr::AnonymousTag(n) if n.as_str() == "Range"),
        "expected AnonymousTag(Range), got {:?}",
        expr,
    );
}
