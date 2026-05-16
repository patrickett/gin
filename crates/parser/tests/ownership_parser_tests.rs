//! Parser tests for the ownership system: `mut`/`own` parameter syntax
//! and `mut expr`/`own expr` call-site argument syntax.

use ast::{Expr, ParamConvention};
use internment::Intern;
use parser::parse_from_str;

#[test]
fn test_parse_own_param() {
    let src = "greet(own s String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("greet")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("s")),
        Some(&ParamConvention::Own)
    );
}

#[test]
fn test_parse_mut_param() {
    let src = "set_value(mut s String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("set_value")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("s")),
        Some(&ParamConvention::Mut)
    );
}

#[test]
fn test_parse_bare_param_is_readonly() {
    let src = "print(s String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("print")).unwrap();
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
}

#[test]
fn test_parse_mixed_params() {
    let src = "process(mut db Database, own txn Transaction, name String):
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("process")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("db")),
        Some(&ParamConvention::Mut)
    );
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("txn")),
        Some(&ParamConvention::Own)
    );
    assert!(
        bind.param_conventions
            .get(&Intern::from_ref("name"))
            .is_none()
    );
}

#[test]
fn test_parse_own_param_with_return_type() {
    let src = "consume(own s String) Int:
    return 0
return
";
    let ast = parse_from_str(src);
    let bind = ast.defs().get(&Intern::from_ref("consume")).unwrap();
    assert_eq!(
        bind.param_conventions.get(&Intern::from_ref("s")),
        Some(&ParamConvention::Own)
    );
    // Return tag should be set (capitalized type annotation)
    assert!(
        bind.return_tag.is_some(),
        "return_tag should be Some for Int return type"
    );
}

#[test]
fn test_parse_own_in_call() {
    // A call with `own x` argument inside a function body
    let src = "\
main:
    x: 42
    dummy(own x)
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
        let has_own_arg = !exprs.iter().all(|expr| {
            if let Expr::FnCall(call) = &expr.value
                && let Some(args) = &call.args
            {
                return !args.iter().any(|a| matches!(&a.value, Expr::OwnArg(_)));
            }
            true
        });
        assert!(has_own_arg, "expected OwnArg in call args");
    }
}

#[test]
fn test_parse_mut_in_call() {
    let src = "\
main:
    x: 42
    set_value(mut x)
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
        let has_mut_arg = exprs.iter().any(|expr| {
            if let Expr::FnCall(call) = &expr.value
                && let Some(args) = &call.args
            {
                return args.iter().any(|a| matches!(&a.value, Expr::MutArg(_)));
            }
            false
        });
        assert!(has_mut_arg, "expected MutArg in call args");
    }
}

#[test]
fn test_parse_attr_lin() {
    let src = "#[lin]
Transaction has (id Int)
";
    let ast = parse_from_str(src);
    let decl = ast.tags().get(&Intern::from_ref("Transaction")).unwrap();
    assert!(decl.attributes.is_lin, "Transaction should be #[lin]");
}

#[test]
fn test_parse_attr_not_copy() {
    let src = "#[not_copy]
Handle has (fd Int)
";
    let ast = parse_from_str(src);
    let decl = ast.tags().get(&Intern::from_ref("Handle")).unwrap();
    assert!(decl.attributes.is_not_copy, "Handle should be #[not_copy]");
}
