mod support;

#[test]
fn named_result_rows_lower_as_multiple_returns() {
    let source = "Byte is in 0 to 255\npair(first Byte, second Byte) first Byte, second Byte: return first, second\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "results.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(
        mlir.contains("function_type = (i8, i8) -> (i8, i8)"),
        "{mlir}"
    );
    assert!(!mlir.contains("llvm.insertvalue"), "{mlir}");
}

#[test]
fn calls_and_named_projection_select_an_ssa_result() {
    let source = "Byte is in 0 to 255\npair(first Byte, second Byte) first Byte, second Byte: return first, second\nsecond(value Byte) Byte:\n    result: pair(value, value)\n    return result.second\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "result_projection.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("callee = @pair"), "{mlir}");
    assert!(mlir.contains(": (i8, i8) -> (i8, i8)"), "{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
}

#[test]
fn one_named_result_uses_scalar_abi() {
    let source = "Byte is in 0 to 255\nid(value Byte) only Byte: return value\n";
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "one_result.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("function_type = (i8) -> i8"), "{mlir}");
}

#[test]
fn library_defined_fixed_array_lowers_as_an_inline_aggregate() {
    let source = r#"
Byte is in 0 to 255
#fixed_array(element, count)
Buffer(element, count BigInt) has
make(value Byte) Buffer(Byte, 4): return [value; 4]
get(value Buffer(Byte, 4), index Byte) Byte: return value.[index]
set(value mut Buffer(Byte, 4), index Byte, replacement Byte): return value.[index]:: replacement
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "fixed_array.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.array"), "{mlir}");
    assert!(mlir.contains("llvm.insertvalue"), "{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("llvm.load"), "{mlir}");
    assert!(
        mlir.contains("function_type = (i8) -> !llvm.array<4 x i8>"),
        "{mlir}"
    );
}

#[test]
fn structural_array_forms_lower_as_inline_aggregates() {
    let source = r#"
Byte is in 0 to 255
explicit(value Byte) [Byte; 4]: return [value, value, value, value]
repeated(value Byte) [Byte; 4]: return [value; 4]
inferred_explicit: [0, 0, 0, 0]
inferred_repeated: [0; 4]
"#;
    let (mlir, diagnostics) = support::codegen_to_mlir_text(source, "structural_array.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.array<4 x i8>"), "{mlir}");
    assert!(mlir.matches("llvm.insertvalue").count() >= 16, "{mlir}");
}

#[test]
fn symbolic_fixed_array_lengths_in_declaration_reach_codegen_boundary_as_error() {
    let source = r#"Int is in 0 to 255
Byte is in 0 to 255
#fixed_array(element, count)
Buffer(element, count Int) has
make(value Byte, n Int) Buffer(Byte, n): return [value; n]
"#;
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let (verified, symptoms) = support::codegen_verifies(source, "symbolic_fixed_array.gin");
    assert!(
        symptoms.iter().any(|diagnostic| {
            diagnostic.code.slug() == "unresolved-fixed-array-construction"
                && diagnostic.message.contains(
                    "fixed-array construction reached codegen without array representation",
                )
        }),
        "expected unresolved fixed-array construction symptom: {:?}",
        symptoms
    );

    assert!(
        !verified,
        "symbolic array must not produce a verified module"
    );
}

#[test]
fn symbolic_fixed_array_length_with_concrete_substitution_lowers_to_array() {
    let source = format!(
        "Int is in 0 to 255\nInt has\n    finish(self):\n        return\n{}#fixed_array(element, count)\nBuffer(element, count Int) has\n    finish(self):\n        return\nmake(value Byte) Buffer(Byte, 3): return [value; 3]\nmain() Buffer(Byte, 3):\n    return make(1)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let parsed = parser::parse_from_str(&source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = typecheck::transform::transform_file(parsed, typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (mlir, symptoms) = support::codegen_to_mlir_text(
        &source,
        "symbolic_fixed_array_concrete_substitution.gin",
        false,
    );
    assert!(
        symptoms.is_empty(),
        "expected concrete array substitution to compile: {symptoms:?}"
    );
    assert!(
        mlir.contains("llvm.array"),
        "concrete fixed-array construction should lower to concrete llvm.array: {mlir}"
    );
}

#[test]
#[allow(unsafe_code)]
fn compound_projected_writes_transfer_values_and_evaluate_the_index_once() {
    let source = r#"
Byte is in 0 to 255
Byte has
    copy(self ref) Self: return deref self
    finish(self):
        return
#fixed_array(element, count)
Buffer(element, count BigInt) has
    finish(self):
        return

Box has
    value Byte, marker Byte
    finish(self):
        return

add_bits(a Byte, b Byte) Byte:
    left is in 0 to 255: a
    right is in 0 to 255: b
    return ((left + right) & 255) as Byte
#infix('+')
#inline
byte_add(a Byte, b Byte) Byte: return add_bits(a, b)
next_index() Byte: return 0
update(value mut Buffer(Byte, 4), replacement Byte) Byte:
    value.[next_index()]+: Byte.copy(replacement)
    return replacement

update_record(value mut Box, replacement Byte) Byte:
    value.value+: Byte.copy(replacement)
    return replacement
"#;
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = typecheck::transform::transform(
        &parsed,
        typecheck::FileId(0),
        &typecheck::transform::TransformCtx::new(),
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "compound_fixed_array.gin", false);

    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert_eq!(mlir.matches("callee = @next_index").count(), 1, "{mlir}");
    assert!(mlir.contains("callee = @byte_add"), "{mlir}");

    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    for export in ["update", "update_record"] {
        let context = codegen::NativeCompiler::create_context();
        let (module, diagnostics) =
            codegen::CodegenContext::build_module_from_resolved_program_for_target(
                &context,
                typed.resolved_program(),
                codegen::CodegenSourceMap::new(
                    "compound_fixed_array.gin",
                    source,
                    &typed.span_table,
                ),
                &target,
            );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let engine = codegen::NativeEngine::new_with_export_abi(module.unwrap(), export).unwrap();
        let mut bytes = [40u8, 2, 3, 4];
        let mut pointer = bytes.as_mut_ptr();
        let mut replacement = 7u8;
        let mut result = 0u8;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut pointer as *mut *mut u8).cast(),
                (&mut replacement as *mut u8).cast(),
                (&mut result as *mut u8).cast(),
            ])
        }
        .unwrap();
        assert_eq!(bytes, [47, 2, 3, 4]);
        assert_eq!(result, replacement);
    }
}
