mod support;

use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use support::typed;
use typecheck::{PlaceId, TypedExprKind};

const SOURCE: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nInner has\n    value Byte\nOuter has\n    inner Inner\nobserve(value ref Byte) Byte: return deref value\nselect(value Outer) Byte:\n    observe(value.inner.value)\n    return value.inner.value\n";

use test_fixtures::host_compile_target as target;

#[test]
fn addressed_places_reject_missing_and_unordered_ancestors() {
    let original = typed(SOURCE);
    let (expression, inner) = original
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| match kind {
            TypedExprKind::Ref(inner) if original.exprs.place_of(*inner).flatten().is_some() => {
                Some((index, *inner))
            }
            _ => None,
        })
        .expect("addressed projected place");
    let place = original.exprs.place_of(inner).flatten().unwrap();
    let parent = original.places[place.0 as usize].parent.unwrap();
    assert!(
        original.places[parent.0 as usize].parent.is_some(),
        "nested projection"
    );
    for corruption in ["missing", "self", "forward", "missing_parent"] {
        let mut typed = original.clone();
        match corruption {
            "missing" => {
                typed.exprs.facts.ownership_facts.place[inner.as_usize()] = Some(PlaceId(u32::MAX))
            }
            "self" => typed.places[place.0 as usize].parent = Some(place),
            "forward" => typed.places[parent.0 as usize].parent = Some(place),
            "missing_parent" => typed.places[place.0 as usize].parent = Some(PlaceId(u32::MAX)),
            _ => unreachable!(),
        }
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("addressed-place.gin", SOURCE, &typed.span_table),
            &target(),
        );
        assert!(module.is_none(), "{corruption}");
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-addressed-place-invalid"
                    && d.span == typed.span_table.get(typed.exprs.span[expression])),
            "{corruption}: {diagnostics:?}"
        );
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.slug() == "codegen-invalid-module")
        );
    }
}

#[test]
#[allow(unsafe_code)]
fn nested_addressed_places_preserve_native_values() {
    let pointer_source = SOURCE
        .replace(
            "observe(value ref Byte) Byte: return deref value",
            "observe(value @Byte) Byte: return value.[0]",
        )
        .replace("observe(value.inner.value)", "observe(@value.inner.value)");
    for source in [SOURCE, pointer_source.as_str()] {
        let typed = typed(source);
        let context = NativeCompiler::create_context();
        let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
            &context,
            typed.resolved_program(),
            CodegenSourceMap::new("addressed-place.gin", source, &typed.span_table),
            &target(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let module = module.unwrap();
        assert!(module.as_operation().to_string().contains("llvm.alloca"));
        let engine = NativeEngine::new_with_export_abi(module, "select").unwrap();
        for mut input in [0u8, 1, 128, 255] {
            let mut output = 0u8;
            unsafe {
                engine.invoke_packed(&mut [
                    (&mut input as *mut u8).cast(),
                    (&mut output as *mut u8).cast(),
                ])
            }
            .unwrap();
            assert_eq!(output, input);
        }
    }
}

#[test]
fn invalid_addressed_context_stops_before_local_storage_lowering() {
    let source = SOURCE
        .replace(
            "    observe(value.inner.value)",
            "    local: value\n    observe(local.inner.value)",
        )
        .replace("return value.inner.value", "return local.inner.value");
    let mut typed = typed(&source);
    let inner = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Ref(inner) if typed.exprs.place_of(*inner).flatten().is_some() => {
                Some(*inner)
            }
            _ => None,
        })
        .expect("addressed projection");
    let local = typed
        .exprs
        .kind
        .iter()
        .position(
            |kind| matches!(kind, TypedExprKind::Bind { name, .. } if name.as_str() == "local"),
        )
        .expect("local binding");
    for expression in [inner.as_usize(), local] {
        typed.exprs.facts.ownership_facts.place[expression] = Some(PlaceId(u32::MAX));
    }
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("invalid-context.gin", &source, &typed.span_table),
        &target(),
    );
    assert!(module.is_none());
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(
        diagnostics[0].code.slug().as_str(),
        "codegen-addressed-place-invalid"
    );
}
