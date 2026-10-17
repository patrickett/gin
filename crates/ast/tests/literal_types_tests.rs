//! Phase 1: `Ty::Literal` site binds vs literal-only `Ty::Union` declared tags.

use ast::ConstValue;
use internment::Intern;
use typecheck::ty::Ty;
use typecheck::{DefId, TagId};

mod support;
use support::transform_source;

fn bind_body_ty(typed: &typecheck::TypedFileAst, name: &str) -> Ty {
    let def_id = DefId::named(Intern::new(name.to_string()));
    let bind = typed.defs.get(&def_id).expect(name);
    let body = match &bind.body {
        typecheck::BindBody::Expr(id) => *id,
        _ => panic!("expected expr body for {name}"),
    };
    typed.exprs.ty[body.as_usize()].clone()
}

#[test]
fn site_string_bind_materializes_as_bytes_not_union() {
    let typed = transform_source("greeting: 'John'\n");
    let ty = bind_body_ty(&typed, "greeting");
    assert!(
        matches!(&ty, Ty::Array { elem, size } if matches!(elem.as_ref(), Ty::AnonymousInteger { validity } if validity.domain().contains(255.into()) && !validity.domain().contains(256.into())) && *size == ast::NormalExpr::from(4)),
        "expected a four-byte array, got {ty:?}"
    );
    assert!(ty.union_literal_values().is_none());
}

#[test]
fn empty_brackets_do_not_infer_a_type_named_list() {
    let typed = transform_source("List(element) has unrelated element\nempty: []\n");
    let ty = bind_body_ty(&typed, "empty");

    assert_ne!(
        ty.type_name().map(|name| name.as_str()),
        Some("List"),
        "empty brackets acquired an unrelated declaration name: {ty:?}",
    );
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
    assert!(matches!(&values[0], ConstValue::Text(s) if s == "debug"));
}

#[test]
fn widened_string_bind_uses_nominal_string() {
    let typed = transform_source("String has pointer @Byte, len Int\n\nlabel String: 'John'\n");
    let ty = bind_body_ty(&typed, "label");
    match typed.type_registry.resolved_definition_for_type(&ty) {
        Ty::Record { name, .. } if name.as_str() == "String" => {}
        other => panic!("widened bind body type should be String, got {other:?}"),
    }
    let bind = typed
        .defs
        .get(&DefId::named(Intern::new("label".to_string())))
        .expect("label");
    match typed
        .type_registry
        .resolved_definition_for_type(&bind.return_type)
    {
        Ty::Record { name, .. } if name.as_str() == "String" => {}
        other => panic!("widened bind return_type should be String, got {other:?}"),
    }
}
