mod support;

#[test]
fn integer_call_arguments_use_parameter_width_and_source_signedness() {
    for (source_domain, target_domain, extension) in [
        ("0 to 255", "0 to 65535", "arith.extui"),
        ("0 to 255", "-32768 to 32767", "arith.extui"),
        ("-128 to 127", "-32768 to 32767", "arith.extsi"),
    ] {
        let source = format!(
            "identity(input is in {target_domain}) is in {target_domain}: return input\nconvert(value is in {source_domain}) is in {target_domain}: return identity(value)\n"
        );
        let (mlir, flaws) = support::codegen_to_mlir_text(&source, "parameter_widening.gin", false);
        assert!(flaws.is_empty(), "{source}: {flaws:?}\n{mlir}");
        assert!(mlir.contains(extension), "{source}: {mlir}");
    }
}

#[test]
fn multi_result_calls_convert_integer_arguments_to_the_parameter_width() {
    let source = "Word is in -32768 to 32767\nWord has\n    finish(self):\n        return\npair(input is in -32768 to 32767) first Word, second Word: return input as Word, input as Word\nconvert(value is in -128 to 127) first Word, second Word: return pair(value)\n";
    let (mlir, flaws) = support::codegen_to_mlir_text(source, "row_parameter_widening.gin", false);
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("arith.extsi"), "{mlir}");
}

#[test]
fn proven_same_width_integer_conversion_emits_no_runtime_cast() {
    let source = "Tiny is in 0 to 15\nconverted: 10 as Tiny";
    let (mlir, symptoms) = support::codegen_to_mlir_text(source, "integer_conversion.gin", false);

    assert!(symptoms.is_empty(), "{symptoms:?}");
    assert!(mlir.contains("i4"), "{mlir}");
    for operation in ["arith.trunci", "arith.extsi", "arith.extui"] {
        assert!(!mlir.contains(operation), "unexpected {operation}:\n{mlir}");
    }
}

#[test]
fn widened_record_field_is_extracted_at_its_storage_width() {
    let source = "Byte is in 0 to 255\nDescriptor has index Byte\nread(value Descriptor) is in 0 to 4294967295:\n    index is in 0 to 4294967295: value.index\n    return index\n";
    let (mlir, flaws) = support::codegen_to_mlir_text(source, "field_width.gin", false);
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("llvm.extractvalue"), "{mlir}");
    assert!(mlir.contains("arith.extui"), "{mlir}");
}

#[test]
fn owned_integer_domain_projection_converts_the_storage_width() {
    for preparation in ["", "    inspect(value)\n"] {
        let source = format!(
            "#bits(16)\nWord is in 0 to 255\nWord.release(self):\n    return\ninspect(value ref Word):\n    return\nextract(value Word) is in Word:\n{preparation}    number is in Word: value\n    return number\n"
        );
        let (mlir, flaws) =
            support::codegen_to_mlir_text(&source, "owned_integer_projection.gin", false);
        assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
        assert!(mlir.contains("arith.trunci"), "{mlir}");
    }
}
