use internment::Intern;

use ast::{apply_symbol_aliases, BindValue, Expr, FileAst, ModPath, SpanId, SymbolAlias};
use parser::parse_source_full;

fn add_symbol_alias(ast: &mut FileAst, alias: &str, root: &str, symbol: &str) {
    ast.symbol_aliases.push(SymbolAlias {
        alias: Intern::from_ref(alias),
        target: ModPath::new(
            Intern::from_ref(root),
            vec![Intern::from_ref(symbol)],
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
        let first_expr = &exprs[0].0;
        assert_alias_fn_call(first_expr, "core", &["true"]);
    } else {
        panic!("unexpected binding for main");
    }
}

#[test]
fn rewrites_type_nominal_alias() {
    let mut ast = FileAst::default();
    ast.exprs
        .push((Expr::TypeNominal(Intern::from_ref("Range"), SpanId::INVALID), SpanId::INVALID));
    add_symbol_alias(&mut ast, "Range", "core", "Range");

    apply_symbol_aliases(&mut ast);

    let expr = &ast.exprs[0].0;
    if let Expr::TypeQualified(path) = expr {
        assert_eq!(path.root, Intern::from_ref("core"));
        assert_eq!(path.segments, vec![Intern::from_ref("Range")]);
    } else {
        panic!("expected TypeQualified expression after aliasing");
    }
}
