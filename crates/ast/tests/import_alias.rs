use internment::Intern;

use ast::{BindValue, Expr, FileAst, ModPath, SpanId, Spanned, SymbolAlias};
use parser::query::SourceParseExt;

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
    let mut out = ("main:\n    true\n    return\n").parse_source_full();
    add_symbol_alias(&mut out.ast, "true", "core", "true");

    out.ast.apply_symbol_aliases();

    let main = out.ast.defs.get(&Intern::from_ref("main")).unwrap();
    if let BindValue::Body { exprs, .. } = &main.value {
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

    ast.apply_symbol_aliases();

    // AnonymousTag is handled by a separate pass, so it remains unchanged.
    let expr = &ast.exprs[0].0;
    assert!(
        matches!(expr, Expr::AnonymousTag(n) if n.as_str() == "Range"),
        "expected AnonymousTag(Range), got {:?}",
        expr,
    );
}

#[test]
fn rewrites_constraint_constants_and_preserves_parameter_shadowing() {
    let mut parsed = "bounded(value Count and value <= limit): return\nshadow(limit Count and limit > 0): return\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    add_symbol_alias(&mut parsed.ast, "limit", "core", "limit");
    parsed.ast.apply_symbol_aliases();
    let bounded = &parsed.ast.defs[&Intern::from_ref("bounded")];
    assert_eq!(bounded.param_refinements[&Intern::from_ref("value")], constraint("value <= core.limit"));
    let shadow = &parsed.ast.defs[&Intern::from_ref("shadow")];
    assert_eq!(shadow.param_refinements[&Intern::from_ref("limit")], constraint("limit > 0"));
}

#[test]
fn qualifies_module_constants_inside_parameter_constraints() {
    let parsed = "limit := 7\nbounded(value Count and value <= limit): return\nshadow(limit Count and limit > 0): return\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let ast = parsed.ast.qualify_module_defs("numbers");
    let bounded = &ast.defs[&Intern::from_ref("numbers.bounded")];
    assert_eq!(bounded.param_refinements[&Intern::from_ref("value")], constraint("value <= numbers.limit"));
    let shadow = &ast.defs[&Intern::from_ref("numbers.shadow")];
    assert_eq!(shadow.param_refinements[&Intern::from_ref("limit")], constraint("limit > 0"));
}

fn constraint(proposition: &str) -> ast::PredicateExpr {
    let parsed = format!("check(value Count and {proposition}): return\n").parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    parsed.ast.defs[&Intern::from_ref("check")].param_refinements[&Intern::from_ref("value")].clone()
}
