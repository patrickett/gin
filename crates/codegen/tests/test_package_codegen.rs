use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine, ResolvedFileInput};
use flask::CompileTarget;
use melior::ir::operation::OperationLike;
use parser::query::SourceParseExt;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

fn test_target() -> CompileTarget {
    CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux-gnu").expect("test target"),
    )
}

#[test]
fn invokes_caller_with_definition_from_another_typed_file() {
    let sources = [
        (
            "provider.gin",
            "Signed64 is in -9223372036854775808 to 9223372036854775807\nhelper() Signed64: 42",
        ),
        ("caller.gin", "answer() Signed64: helper()"),
    ];
    let mut asts: Vec<_> = sources
        .iter()
        .map(|(_, source)| source.parse_source_full().ast)
        .collect();
    for ast in &mut asts {
        assert!(
            typecheck::prepare_parse_ast(ast, &test_target()).is_empty(),
            "package preparation should succeed"
        );
    }
    let artifacts = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(test_target()),
    );
    for typed in &artifacts.typed_asts {
        assert!(typed.all_flaws().is_empty(), "typecheck should succeed");
    }

    let inputs: Vec<_> = artifacts
        .typed_asts
        .iter()
        .zip(sources)
        .map(|(typed, (filename, source))| {
            ResolvedFileInput::new(
                typed.resolved_program(),
                CodegenSourceMap::new(filename, source, &typed.span_table),
            )
        })
        .collect();
    let context = NativeCompiler::create_context();
    let output = CodegenContext::build_module_from_resolved_files_for_target(
        &context,
        &inputs,
        &test_target(),
    );
    assert!(
        output
            .diagnostics
            .iter()
            .all(|file| file.diagnostics.is_empty()),
        "codegen should have no diagnostics"
    );
    let module = output.module.expect("combined module");
    assert!(
        module.as_operation().verify(),
        "combined module should verify"
    );
    let mlir = module.as_operation().to_string();
    assert!(mlir.contains("@helper"));
    assert!(mlir.contains("@answer"));

    let engine = NativeEngine::new(module, "answer").expect("native engine");
    assert_eq!(engine.invoke_i64().expect("native result"), 42);
}

#[test]
fn ordinary_call_uses_the_defined_signature() {
    let sources = [
        (
            "provider.gin",
            r#"Signed64 is in -9223372036854775808 to 9223372036854775807
external(value Signed64) Signed64: value"#,
        ),
        ("caller.gin", "answer() Signed64: external(42)"),
    ];
    let mut asts: Vec<_> = sources
        .iter()
        .map(|(_, source)| source.parse_source_full().ast)
        .collect();
    for ast in &mut asts {
        assert!(typecheck::prepare_parse_ast(ast, &test_target()).is_empty());
    }
    let artifacts = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(test_target()),
    );
    assert!(
        artifacts
            .typed_asts
            .iter()
            .all(|typed| typed.all_flaws().is_empty()),
        "{:?}",
        artifacts
            .typed_asts
            .iter()
            .flat_map(typecheck::TypedFileAst::all_flaws)
            .collect::<Vec<_>>()
    );
    let inputs: Vec<_> = artifacts
        .typed_asts
        .iter()
        .zip(sources)
        .map(|(typed, (filename, source))| {
            ResolvedFileInput::new(
                typed.resolved_program(),
                CodegenSourceMap::new(filename, source, &typed.span_table),
            )
        })
        .collect();
    let context = NativeCompiler::create_context();
    let output = CodegenContext::build_module_from_resolved_files_for_target(
        &context,
        &inputs,
        &test_target(),
    );
    let module = output.module.expect("verified module");
    let mlir = module.as_operation().to_string();
    let declaration = mlir
        .lines()
        .find(|line| line.contains("func.func") && line.contains("@external"))
        .expect("function definition");

    assert!(declaration.contains("-> i64"), "{declaration}");
    assert!(declaration.contains('{'), "{declaration}");
}

#[test]
fn duplicate_function_symbols_return_an_invalid_module_diagnostic() {
    let sources = [
        (
            "first.gin",
            "Signed64 is in -9223372036854775808 to 9223372036854775807\nduplicate() Signed64: 1",
        ),
        ("second.gin", "duplicate() Signed64: 2"),
    ];
    let asts = sources
        .iter()
        .map(|(_, source)| source.parse_source_full().ast)
        .collect();
    let artifacts = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(test_target()),
    );
    let inputs: Vec<_> = artifacts
        .typed_asts
        .iter()
        .zip(sources)
        .map(|(typed, (filename, source))| {
            ResolvedFileInput::new(
                typed.resolved_program(),
                CodegenSourceMap::new(filename, source, &typed.span_table),
            )
        })
        .collect();
    let context = NativeCompiler::create_context();
    let output = CodegenContext::build_module_from_resolved_files_for_target(
        &context,
        &inputs,
        &test_target(),
    );

    assert!(output.module.is_none());
    assert!(
        output.diagnostics[0]
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "codegen-invalid-module")
    );
}

#[test]
fn string_globals_use_deterministic_internal_namespaces() {
    let sources = [
        (
            "first.gin",
            r#"Signed64 is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
String has pointer @Byte, len Signed64
first() String: String(pointer: (@'first') as @Byte, len: 5)
answer() Signed64: 42"#,
        ),
        (
            "second.gin",
            r#"Byte is in 0 to 255
String has pointer @Byte, len Signed64
second() String: String(pointer: (@'second') as @Byte, len: 6)"#,
        ),
    ];
    let asts = sources
        .iter()
        .map(|(_, source)| source.parse_source_full().ast)
        .collect();
    let artifacts = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(test_target()),
    );
    let inputs: Vec<_> = artifacts
        .typed_asts
        .iter()
        .zip(sources)
        .map(|(typed, (filename, source))| {
            ResolvedFileInput::new(
                typed.resolved_program(),
                CodegenSourceMap::new(filename, source, &typed.span_table),
            )
        })
        .collect();
    let context = NativeCompiler::create_context();
    let output = CodegenContext::build_module_from_resolved_files_for_target(
        &context,
        &inputs,
        &test_target(),
    );
    let module = output.module.expect("verified module");
    let mlir = module.as_operation().to_string();

    assert!(mlir.contains("__gin-string-0_0"), "{mlir}");
    assert!(mlir.contains("__gin-string-1_0"), "{mlir}");
    let engine = NativeEngine::new(module, "answer").expect("native engine");
    assert_eq!(engine.invoke_i64().expect("native result"), 42);
}
