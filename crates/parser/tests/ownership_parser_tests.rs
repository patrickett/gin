//! Parser tests for the ownership system: `~` consume-parameter syntax
//! and `~expr` call-site consume argument syntax.

use ast::{Expr, ParamConvention};
use internment::Intern;
use parser::parse_from_str;

#[test]
fn test_parse_consume_param() {
    let src = "greet(~s String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("greet")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("s")),
        Some(&ParamConvention::Consume)
    );
}

#[test]
fn test_parse_bare_param_is_inferred_by_default() {
    let src = "print(s String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("print")).unwrap();
    // Bare params default to Inferred; the parser only stores non-default conventions,
    // so nothing should be in the map for this param.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
}

#[test]
fn test_parse_mixed_params() {
    let src = "process(~db Database, txn Transaction, name String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("db")),
        Some(&ParamConvention::Consume)
    );
    // Bare params (Inferred, default) are not stored in the map.
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("txn"))
            .is_none()
    );
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("name"))
            .is_none()
    );
}

#[test]
fn test_parse_own_param_with_return_type() {
    let src = "consume(s String) Int:
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("consume")).unwrap();
    // Bare param defaults to Inferred; convention not stored in the map.
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
    // Return tag should be set (capitalized type annotation)
    assert!(
        bind.return_tag.is_some(),
        "return_tag should be Some for Int return type"
    );
}

#[test]
fn test_parse_consume_arg_in_call() {
    // A call with `~x` argument inside a function body
    let src = "\
main:
    x: 42
    dummy(~x)
    return 0
return
";
    let ast = parse_from_str(src);
    assert!(
        ast.defs().contains_key(&Intern::from_ref("main")),
        "main should be a def"
    );
    let main_def = ast.defs().get(&Intern::from_ref("main")).unwrap();
    if let ast::BindValue::Body { exprs, .. } = main_def.value() {
        let has_consume_arg = exprs.iter().any(|expr| {
            if let Expr::FnCall(call) = &expr.value
                && let Some(args) = &call.args
            {
                return args.iter().any(|a| matches!(&a.value, Expr::ConsumeArg(_)));
            }
            false
        });
        assert!(has_consume_arg, "expected ConsumeArg in call args");
    }
}

#[test]
fn test_parse_and_is_not_copy() {
    // `and is not Copy` opts a type into linear semantics.
    let src = "Transaction has (id Int)\n     and is not Copy\n";
    let ast = parse_from_str(src);
    let decl = ast.tags().get(&Intern::from_ref("Transaction")).unwrap();
    let binding = decl.marker_bindings.first().unwrap();
    assert_eq!(binding.marker_name.as_str(), "Copy");
    assert!(!binding.positive);
}
