mod support;

#[test]
fn constructors_and_patterns_use_declared_discriminants_at_their_storage_width() {
    for (attribute, variants, expected) in [
        ("", "Foo := 300 or Bar := -2", "i10"),
        ("#bits(16)\n", "Foo := 1 or Bar := 2", "i16"),
        ("", "Foo := 18446744073709551616 or Bar := 1", "i65"),
        (
            "",
            "Only := 340282366920938463463374607431768211455",
            "i128",
        ),
    ] {
        let source = format!(
            "{attribute}Code is {variants}\nmake() Code: return Foo\nread(value Code) is in 0 to 1: return when value is Foo then 1 else 0\n"
        );
        let source = if variants.starts_with("Only") {
            source.replace("Foo", "Only")
        } else {
            source
        };
        let (mlir, flaws) =
            support::codegen_to_mlir_text(&source, "union_discriminants.gin", false);
        assert!(flaws.is_empty(), "{source}: {flaws:?}\n{mlir}");
        assert!(mlir.contains(expected), "{mlir}");
        assert!(
            !mlir.contains("arith.extui"),
            "pattern comparison widened the discriminant: {mlir}"
        );
    }
}

#[test]
fn signed_discriminant_conversion_extends_the_code_with_its_sign() {
    let source = "Word is in -32768 to 32767\nWord.finish(self): return\nCode is Foo := 300 or Bar := -2\nconvert(value Code) Word: return value as Word\n";
    let (mlir, flaws) = support::codegen_to_mlir_text(source, "signed_discriminant.gin", false);
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("arith.extsi"), "{mlir}");
}

#[test]
fn guarded_integer_conversion_materializes_the_declared_code() {
    let source = "Code is Foo := 1 or Bar := 100\nconvert(value is in 0 to 255) Code: return when value = 100 then value as Code else Foo\n";
    let (mlir, flaws) = support::codegen_to_mlir_text(source, "checked_discriminant.gin", false);
    assert!(flaws.is_empty(), "{flaws:?}\n{mlir}");
    assert!(mlir.contains("arith.trunci"), "{mlir}");
}
