use backend::{NativeCompiler, Profile};

#[test]
fn unresolved_runtime_symbol_is_not_synthesized() {
    let directory = tempfile::tempdir().unwrap();
    let object = directory.path().join("runtime caller.o");
    let executable = directory.path().join("runtime caller");
    let compiler = NativeCompiler::default();
    let (emitted, diagnostics) = compiler.native_from_mlir_text(
        r#"module {
            func.func private @runtime_print_int(i64) -> i64
            func.func @main() -> i32 {
                %value = arith.constant 17 : i64
                %result = func.call @runtime_print_int(%value) : (i64) -> i64
                %status = arith.trunci %result : i64 to i32
                return %status : i32
            }
        }"#,
        &object,
        Profile::Debug,
        None,
    );
    assert!(emitted, "{diagnostics:?}");
    assert!(object.is_file());

    let (linked, diagnostics) = NativeCompiler::link_executable(&object, &executable, None);
    assert!(!linked, "an unresolved runtime symbol must fail to link");
    assert!(!diagnostics.is_empty());
    let message = format!("{diagnostics:?}");
    assert!(message.contains("Linking failed"), "{message}");
    assert!(message.contains("runtime_print_int"), "{message}");
}

#[test]
fn self_contained_main_links_and_runs() {
    let directory = tempfile::tempdir().unwrap();
    let object = directory.path().join("standalone main.o");
    let executable = directory.path().join("standalone main");
    let compiler = NativeCompiler::default();
    let (emitted, diagnostics) = compiler.native_from_mlir_text(
        r#"module {
            func.func @main() -> i32 {
                %status = arith.constant 23 : i32
                return %status : i32
            }
        }"#,
        &object,
        Profile::Debug,
        None,
    );
    assert!(emitted, "{diagnostics:?}");
    assert!(object.is_file());

    let (linked, diagnostics) = NativeCompiler::link_executables(&[&object], &executable, None);
    assert!(linked, "{diagnostics:?}");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let result = std::process::Command::new(&executable).output().unwrap();
    assert_eq!(result.status.code(), Some(23));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
}
