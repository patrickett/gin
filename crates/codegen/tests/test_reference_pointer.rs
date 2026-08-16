mod support;

fn function_type_of<'a>(mlir: &'a str, symbol: &str) -> Option<&'a str> {
    mlir.lines().find_map(|line| {
        if !line.contains(&format!("sym_name = \"{symbol}\"")) {
            return None;
        }
        let start = line.find("function_type = ")? + "function_type = ".len();
        let end = line[start..]
            .find(", sym_name")
            .map(|offset| start + offset)
            .unwrap_or(line.len());
        Some(line[start..end].trim())
    })
}

#[test]
fn named_and_compatibility_references_share_pointer_type() {
    let source = r#"Word is in 0...255
RefWord is ref Word
#intrinsic(AddressOffset)
move_named(address RefWord, index Word) RefWord extern
#intrinsic(AddressOffset)
move_compat(address ref Word, index Word) ref Word extern
entry(value Word) Word:
    ref_value: ref Word: ref value
    named_ptr: RefWord: move_named(ref_value, 1)
    compat_ptr: ref Word: move_compat(ref_value, 1)
    return value
"#;
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "reference_pointer_offset.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert!(mlir.contains("llvm.getelementptr"), "{mlir}");
    assert!(mlir.contains("sym_name = \"move_named\""));
    assert!(mlir.contains("sym_name = \"move_compat\""));
    assert_eq!(
        function_type_of(&mlir, "move_named"),
        function_type_of(&mlir, "move_compat"),
    );
    assert_eq!(
        mlir.matches("function_type = (!llvm.ptr, i8) -> !llvm.ptr")
            .count(),
        2,
        "{mlir}
",
    );
}

#[test]
fn named_and_compatibility_reference_loads_lower_to_pointer_loads() {
    let source = r#"Word is in 0...255
RefWord is ref Word
#intrinsic(AddressLoad)
load_named(address RefWord) Word extern
#intrinsic(AddressLoad)
load_compat(address ref Word) Word extern
entry(value Word) Word:
    ref_value: ref Word: ref value
    named: Word: load_named(ref_value)
    compat: Word: load_compat(ref_value)
    return named
"#;
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "reference_pointer_load.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert_eq!(
        mlir.matches("function_type = (!llvm.ptr) -> i8").count(),
        2,
        "{mlir}
    ",
    );
    assert_eq!(
        function_type_of(&mlir, "load_named"),
        function_type_of(&mlir, "load_compat"),
    );
    assert!(
        mlir.matches("llvm.load").count() >= 2,
        "{mlir}
",
    );
}

#[test]
fn generic_reference_alias_and_compat_reference_invoke_identical_pointer_ops() {
    let source = r#"Word is in 0...255
RefAlias(x) is ref x
#intrinsic(AddressOffset)
move_named(address RefAlias(Word), index Word) RefAlias(Word) extern
#intrinsic(AddressOffset)
move_compat(address ref Word, index Word) ref Word extern
#intrinsic(AddressLoad)
load_named(address RefAlias(Word)) Word extern
#intrinsic(AddressLoad)
load_compat(address ref Word) Word extern
entry(value Word) Word:
    ref_value: ref Word: ref value
    alias_ptr: RefAlias(Word): move_named(ref_value, 1)
    _ = load_named(alias_ptr)
    compat_ptr: ref Word: move_compat(ref_value, 1)
    _ = load_compat(compat_ptr)
    return value
"#;
    let (mlir, diagnostics) =
        support::codegen_to_mlir_text(source, "reference_pointer_generic_alias.gin", false);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{mlir}");
    assert_eq!(
        function_type_of(&mlir, "move_named"),
        function_type_of(&mlir, "move_compat"),
    );
    assert_eq!(
        function_type_of(&mlir, "load_named"),
        function_type_of(&mlir, "load_compat"),
    );
    assert!(mlir.contains("llvm.getelementptr"));
    assert!(mlir.contains("llvm.load"));
}
