use ast::DefinitionId;
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn preserves_each_function_body_and_source_name() {
    let parsed = "choose(value A): return value\nchoose(value B): return value\nchoose(value C): return value\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_eq!(parsed.ast.defs.len(), 3);
    let name = Intern::from_ref("choose");
    assert_eq!(parsed.ast.defs.groups()[&name].len(), 3);
    assert_eq!(parsed.ast.public_def_names(), vec![name]);
    for index in 0..3 {
        let bind = &parsed.ast.defs[&DefinitionId::member(name, index)];
        assert_eq!(bind.name, name);
    }
}

#[test]
fn private_overloads_stay_private_after_module_qualification() {
    let parsed = "private\nchoose(value A): return value\nchoose(value B): return value\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_eq!(parsed.ast.defs.len(), 2);
    let imported = parsed
        .ast
        .qualify_module_defs("sample")
        .strip_private_for_importer();
    assert!(imported.defs.is_empty());
    assert!(imported.defs.groups().is_empty());
}

#[test]
fn value_declarations_followed_by_initializers_keep_existing_semantics() {
    let parsed = "value Byte\nvalue := 3\n".parse_source_full();
    assert_eq!(parsed.ast.defs.len(), 1);
}

#[test]
fn qualification_preserves_explicit_call_target_members() {
    let mut parsed =
        "choose(value A): return value\nchoose(value B): return value\nresult: choose(1)\n".parse_source_full();
    let result = Intern::from_ref("result");
    let ast::BindValue::Expr(expr) = &mut parsed.ast.defs.get_mut(&result).unwrap().value else {
        panic!("expected expression body");
    };
    let ast::Expr::FnCall(call) = &mut expr.value else {
        panic!("expected function call");
    };
    call.definition = Some(DefinitionId::member(Intern::from_ref("choose"), 1));
    let qualified = parsed.ast.qualify_module_defs("sample");
    let ast::BindValue::Expr(expr) = &qualified.defs[&Intern::from_ref("sample.result")].value
    else {
        panic!("expected expression body");
    };
    let ast::Expr::FnCall(call) = &expr.value else {
        panic!("expected function call");
    };
    assert_eq!(
        call.definition,
        Some(DefinitionId::member(Intern::from_ref("sample.choose"), 1))
    );
}
