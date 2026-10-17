use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine, ResolvedFileInput};
use melior::ir::operation::OperationLike;
use parser::query::SourceParseExt;
use test_fixtures::host_compile_target as test_target;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

#[test]
fn imported_record_literals_publish_their_full_return_signature() {
    let sources = [
        (
            "registers.gin",
            "flags := [architecture: 'aarch64', bank: 'flags', index: 0, width: 4]",
        ),
        (
            "caller.gin",
            concat!(
                "clobbers := [flags]\n",
                "answer() is in -9223372036854775808 to 9223372036854775807:\n",
                "    return flags.width\n",
            ),
        ),
    ];
    assert_native_answer(&sources, 4);
    assert_native_answer(&[sources[1], sources[0]], 4);
}

#[test]
fn imported_nested_sequences_preserve_arrays_and_integer_widths() {
    let sources = [
        (
            "registers.gin",
            "flags := [architecture: 'x86_64', details: [bank: 'flags', index: 0, width: 64]]",
        ),
        (
            "caller.gin",
            concat!(
                "clobbers := [flags, flags]\n",
                "answer() is in -9223372036854775808 to 9223372036854775807:\n",
                "    return flags.details.width\n",
            ),
        ),
    ];
    assert_native_answer(&sources, 64);
    assert_native_answer(&[sources[1], sources[0]], 64);
}

#[allow(unsafe_code)]
fn assert_native_answer(sources: &[(&str, &str)], expected: i64) {
    let asts = sources
        .iter()
        .map(|(_, source)| {
            let mut ast = source.parse_source_full().ast;
            assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
            assert!(typecheck::prepare_parse_ast(&mut ast, &test_target()).is_empty());
            ast
        })
        .collect();
    let artifacts = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(test_target()),
    );
    for typed in &artifacts.typed_asts {
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
    let inputs: Vec<_> = artifacts
        .typed_asts
        .iter()
        .zip(sources)
        .map(|(typed, (filename, source))| {
            ResolvedFileInput::new(
                typed.resolved_program(),
                CodegenSourceMap::new(*filename, source, &typed.span_table),
            )
        })
        .collect();
    let context = NativeCompiler::create_context();
    let output = CodegenContext::build_module_from_resolved_files_for_target(
        &context,
        &inputs,
        &test_target(),
    );
    for file in &output.diagnostics {
        assert!(file.diagnostics.is_empty(), "{:?}", file.diagnostics);
    }
    let module = output.module.expect("verified cross-file module");
    assert!(module.as_operation().verify());
    let engine = NativeEngine::new(module, "answer").expect("native engine");
    assert_eq!(
        unsafe { engine.invoke_i64() }.expect("native result"),
        expected
    );
}
