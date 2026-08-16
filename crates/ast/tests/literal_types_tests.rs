//! Phase 1: `Ty::Literal` site binds vs literal-only `Ty::Union` declared tags.

use ast::ConstValue;
use internment::Intern;
use typecheck::ty::Ty;
use typecheck::{DefId, TagId};

mod support;
use support::transform_source;

fn bind_body_ty(typed: &typecheck::TypedFileAst, name: &str) -> Ty {
    let def_id = DefId(Intern::new(name.to_string()));
    let bind = typed.defs.get(&def_id).expect(name);
    let body = match &bind.body {
        typecheck::BindBody::Expr(id) => *id,
        _ => panic!("expected expr body for {name}"),
    };
    typed.exprs.ty[body.as_usize()].clone()
}

#[test]
fn site_string_bind_is_literal_not_union() {
    let typed = transform_source("greeting: 'John'\n");
    let ty = bind_body_ty(&typed, "greeting");
    assert!(
        matches!(&ty, Ty::Literal(ConstValue::String(s)) if s == "John"),
        "site bind should be Ty::Literal, got {ty:?}"
    );
    assert!(!ty.is_literal_shape() || ty.as_literal_const().is_some());
    assert!(ty.union_literal_values().is_none());
}

#[test]
fn declared_log_level_is_literal_union() {
    let typed = transform_source("LogLevel is 'debug' or 'info'\n");
    let tag = typed
        .tags
        .get(&TagId(Intern::new("LogLevel".to_string())))
        .expect("LogLevel");
    let definition = typed
        .type_registry
        .resolved_definition_for_type(&tag.resolved_ty);
    let values = definition.union_literal_values().expect("literal union");
    assert_eq!(values.len(), 2);
    assert!(matches!(&values[0], ConstValue::String(s) if s == "debug"));
}

#[test]
fn widened_string_bind_uses_nominal_string() {
    let typed =
        transform_source("String has pointer Pointer(Byte), len Int\n\nlabel String: 'John'\n");
    let ty = bind_body_ty(&typed, "label");
    match typed.type_registry.resolved_definition_for_type(&ty) {
        Ty::Record { name, .. } if name.as_str() == "String" => {}
        other => panic!("widened bind body type should be String, got {other:?}"),
    }
    let bind = typed
        .defs
        .get(&DefId(Intern::new("label".to_string())))
        .expect("label");
    match typed
        .type_registry
        .resolved_definition_for_type(&bind.return_type)
    {
        Ty::Record { name, .. } if name.as_str() == "String" => {}
        other => panic!("widened bind return_type should be String, got {other:?}"),
    }
}
