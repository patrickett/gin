mod support;

use ast::Ty;
use internment::Intern;
use support::{body, check};
use typecheck::DefId;

#[test]
fn slice_fields_preserve_declared_types() {
    let typed = check(
        r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
pointer(text Slice(ViewByte)): text.ptr
length(text Slice(ViewByte)): text.len
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(
        typed.exprs.ty_of(body(&typed, "pointer")),
        Some(&Ty::Ptr {
            inner: Box::new(
                typed.tag_types[&typecheck::TagId(Intern::from_ref("ViewByte"))].clone()
            ),
        })
    );
    assert_eq!(
        typed.exprs.ty_of(body(&typed, "length")),
        Some(&typed.tag_types[&typecheck::TagId(Intern::from_ref("ViewLength"))])
    );
}

#[test]
fn text_slice_fields_convert_to_library_defined_pointer_and_size_types() {
    let typed = check(
        r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
Byte is in 0 to 255
Size is in 0 to 18446744073709551615
pointer(text Slice(ViewByte)) @Byte: text.ptr as @Byte
length(text Slice(ViewByte)) Size: text.len as Size
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn text_slice_unknown_field_is_rejected() {
    let typed = check(
        r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
unknown(text Slice(ViewByte)): text.capacity
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-unknown-field" })
    );
}

#[test]
fn text_slice_length_counts_utf8_bytes_at_compile_time() {
    let typed = check(
        r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
length:
    text := 'é🦀'
return #size(text)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let value = typecheck::Evaluator::new(
        typed.resolved_program(),
        typecheck::CompileTimeHost::new(&target),
    )
    .eval(body(&typed, "length"))
    .expect("length evaluation");
    assert!(
        matches!(value, typecheck::EvalOutcome::Complete(ast::ConstValue::Int(bytes)) if bytes == 6.into())
    );
}

#[test]
fn inferred_self_uses_the_receiver_type_and_value_parameter_kind() {
    let typed = check(
        "Word is in 0 to 255\nItem has value Word\nItem.read(self ref) Word: self.value\nItem.release(self own):\n    Item(value: value) := self\nreturn\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for name in ["Item.read", "Item.release"] {
        let bind = &typed.defs[&DefId(Intern::from_ref(name))];
        assert!(matches!(bind.param_kinds[0], ast::ParamKind::Value(_)));
        assert!(!matches!(bind.params[0].1, Ty::Opaque(_) | Ty::Unit));
    }
}
