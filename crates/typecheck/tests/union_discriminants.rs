use ast::{ConstValue, Ty};
use internment::Intern;
use support::check_parsed;
use typecheck::TypedFileAst;

mod support;

fn valid(source: &str) -> TypedFileAst {
    let typed = check_parsed(source);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
    typed
}

#[test]
fn numbered_unions_retain_nominal_identity_and_unit_semantics() {
    let typed = valid("Code is Foo := 3 or Bar := 2 or Baz := 1\nvalue Code: Foo\n");
    let tag = &typed.tags[&typecheck::TagId(Intern::from_ref("Code"))];
    assert!(
        typed
            .type_registry
            .integer_validity_for_type(&tag.resolved_ty)
            .is_none()
    );
    let Ty::Union { variants, .. } = typed
        .type_registry
        .resolved_definition_for_type(&tag.resolved_ty)
    else {
        panic!()
    };
    assert_eq!(
        variants
            .iter()
            .map(|v| v.discriminant.unwrap())
            .collect::<Vec<_>>(),
        [3.into(), 2.into(), 1.into()]
    );
}

#[test]
fn duplicate_discriminants_report_both_assignments() {
    let typed = check_parsed("Code is Foo := 1 or Bar := 1\n");
    let flaws = typed.all_flaws();
    let diagnostic = &flaws
        .iter()
        .find(|(_, flaw)| flaw.code.slug() == "type-duplicate-union-discriminant")
        .unwrap()
        .1;
    assert_eq!(
        diagnostic.message,
        "discriminant value `1` assigned more than once"
    );
    assert_eq!(diagnostic.related().len(), 1);
}

#[test]
fn numbered_unions_require_every_variant_to_have_a_code() {
    for source in ["Code is Foo := 1 or Bar\n", "Code is Foo or Bar := 1\n"] {
        let typed = check_parsed(source);
        assert!(
            support::flaw_codes(&typed).contains(&"type-incomplete-union-discriminants".to_owned())
        );
    }
}

#[test]
fn payloads_and_existing_type_alternatives_cannot_have_discriminants() {
    for source in [
        "Word is in 0 to 255\nWord.finish(self): return\nCode is Foo(value Word) := 1 or Bar := 2\n",
        "Word is in 0 to 255\nCode is Word := 1 or Bar := 2\n",
        "Empty has\n    finish(self):\n        return\nCode is Empty := 1 or Bar := 2\n",
    ] {
        let typed = check_parsed(source);
        assert!(
            support::flaw_codes(&typed)
                .contains(&"type-discriminant-requires-unit-union".to_owned()),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn numeric_conversion_preserves_declared_values_at_compile_time() {
    let typed = valid(
        "Word is in -32768 to 32767\nWord.finish(self): return\nCode is Foo := 300 or Bar := -2\nnumber: Code.Bar as Word\nvariant: 300 as Code\n",
    );
    assert_eq!(
        typed
            .exprs
            .const_value_of(support::body(&typed, "number"))
            .unwrap(),
        &Some(ConstValue::Int((-2).into()))
    );
    assert!(
        matches!(typed.exprs.const_value_of(support::body(&typed, "variant")).unwrap(), Some(ConstValue::Tag { name, .. }) if name.as_str() == "Foo")
    );
}

#[test]
fn numeric_conversion_requires_membership_and_no_truncation() {
    for (source, code) in [
        (
            "Code is Foo := 1 or Bar := 100\nvalue: 2 as Code\n",
            "type-integer-narrowing-failed",
        ),
        (
            "Code is Foo := 1 or Bar := 100\nconvert(value is in 0 to 255) Code: return value as Code\n",
            "type-integer-narrowing-failed",
        ),
        (
            "Byte is in 0 to 255\nCode is Foo := 300 or Bar := 400\nconvert(value Code) Byte: return value as Byte\n",
            "type-integer-narrowing-failed",
        ),
    ] {
        let typed = check_parsed(source);
        assert!(
            support::flaw_codes(&typed).contains(&code.to_owned()),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn guarded_integer_conversion_proves_sparse_membership() {
    valid(
        "Code is Foo := 1 or Bar := 100\nconvert(value is in 0 to 255) Code: return when value = 100 then value as Code else Foo\n",
    );
}

#[test]
fn numeric_round_trip_preserves_sparse_discriminant_knowledge() {
    valid(
        "Word is in -32768 to 32767\nWord.finish(self): return\nCode is Foo := 300 or Bar := -2\nround_trip(value Code) Code: return (value as Word) as Code\n",
    );
}

#[test]
fn rebinding_an_integer_invalidates_its_previous_valid_discriminant() {
    let typed = check_parsed(
        "Code is Foo := 1 or Bar := 100\nconvert Code:\n    value is in 0 to 255: 1\n    value :: 2\n    return value as Code\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code.starts_with("type-integer-narrowing-")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn evaluator_round_trips_runtime_arguments_by_declared_code() {
    let typed = valid(
        "Word is in -32768 to 32767\nWord.finish(self): return\nCode is Foo := 300 or Bar := -2\nconvert(value is in 300 to 300) Word: return (value as Code) as Word\n",
    );
    let target = flask::CompileTarget::Library;
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    let result = evaluator
        .call(
            typecheck::DefId::named(Intern::from_ref("convert")),
            &[ConstValue::Int(300.into())],
        )
        .unwrap();
    assert_eq!(result, ConstValue::Int(300.into()));
}

#[test]
fn numbered_unions_can_be_payloads_of_an_ordinary_union() {
    valid(
        "Code is Foo := 300 or Bar := -2\nOuter is Wrapped(value Code) or Empty\nOuter.finish(self): return\nread(value Outer) is in 0 to 1: return when value is\n    Wrapped(code) then when code is Foo then 1 else 0\n    Empty then 0\n",
    );
}

#[test]
fn matching_remains_exhaustive_over_names() {
    valid(
        "Code is Foo := -2 or Bar := 100\nread(value Code) is in 0 to 1: return when value is\n    Foo then 0\n    Bar then 1\n",
    );
    let typed = check_parsed(
        "Code is Foo := -2 or Bar := 100\nread(value Code) is in 0 to 1: return when value is\n    Foo then 0\n",
    );
    assert!(support::flaw_codes(&typed).contains(&"type-missing-else-arm".to_owned()));
}

#[test]
fn codes_do_not_enable_implicit_integer_operations_or_conversion_between_unions() {
    for body in ["value + 1", "value = 1", "value as Other"] {
        let typed = check_parsed(&format!(
            "Code is Foo := 1 or Bar := 2\nOther is One := 1 or Two := 2\ninvalid(value Code): return {body}\n"
        ));
        assert!(!typed.all_flaws().is_empty(), "accepted {body}");
    }
}

#[test]
fn bits_attribute_controls_discriminant_storage_without_integer_semantics() {
    let typed = valid("#bits(16)\nCode is Foo := 1 or Bar := 2\n");
    let ty = &typed.tags[&typecheck::TagId(Intern::from_ref("Code"))].resolved_ty;
    let repr = typecheck::representation::Repr::derive(ty, Some(&typed.type_registry)).unwrap();
    let layout = typecheck::layout::TargetLayout::x86_64()
        .layout_repr(&repr)
        .unwrap();
    assert_eq!(layout.size, 2);
    assert!(matches!(
        layout.kind,
        typecheck::layout::LayoutKind::Sum {
            discriminant: typecheck::representation::Repr::Bits { width: 16 },
            ..
        }
    ));
    assert!(typed.type_registry.integer_validity_for_type(ty).is_none());
}

#[test]
fn bits_attribute_rejects_discriminants_that_do_not_fit() {
    for source in [
        "#bits(8)\nCode is Foo := 300 or Bar := 1\n",
        "#bits(8)\nCode is Foo := -129 or Bar := 1\n",
        "#bits(0)\nCode is Foo := 1 or Bar := 2\n",
        "#bits(129)\nCode is Foo := 1 or Bar := 2\n",
    ] {
        let typed = check_parsed(source);
        assert!(!typed.all_flaws().is_empty(), "accepted {source}");
    }
}

#[test]
fn very_large_discriminants_use_existing_integer_runtime_limits() {
    valid("Code is Only := 340282366920938463463374607431768211455\n");
    let typed = check_parsed("Code is Only := 340282366920938463463374607431768211456\n");
    assert!(
        support::flaw_codes(&typed).contains(&"type-integer-runtime-width-unsupported".to_owned())
    );
}
