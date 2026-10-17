use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use diagnostic::Diagnostic;
use melior::Context;
use melior::dialect::DialectRegistry;
use melior::ir::operation::{OperationLike, OperationPrintingFlags};
use melior::utility;
use parser::parse_from_str;
use typecheck::transform::{TransformCtx, transform, transform_file};
use typecheck::{FileId, TypedFileAst};

fn test_target() -> flask::CompileTarget {
    flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux-gnu").expect("test target"),
    )
}

#[cfg(test)]
#[test]
fn support_helpers_smoke() {
    let _codegen_to_mlir_text: fn(&str, &str, bool) -> (String, Vec<Diagnostic>) =
        codegen_to_mlir_text;
    let _codegen_to_mlir_text_with_package_context: fn(&str, &str) -> (String, Vec<Diagnostic>) =
        codegen_to_mlir_text_with_package_context;
    let _codegen_verifies: fn(&str, &str) -> (bool, Vec<Diagnostic>) = codegen_verifies;
    let _codegen_to_llvm_ir: fn(&str, &str) -> (String, Vec<Diagnostic>) = codegen_to_llvm_ir;
    let _ = (
        _codegen_to_mlir_text,
        _codegen_to_mlir_text_with_package_context,
        _codegen_verifies,
        _codegen_to_llvm_ir,
    );
}

pub fn codegen_to_llvm_ir(source: &str, filename: &str) -> (String, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let typed = transform_file(ast, FileId(0));
    let context = NativeCompiler::create_context();
    let (module, mut diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &test_target(),
    );
    let Some(module) = module else {
        return (String::new(), diagnostics);
    };
    let compiler = NativeCompiler::default();
    let (llvm, backend_diagnostics) =
        compiler.llvm_ir_from_module(&module, codegen::Profile::Debug, false);
    diagnostics.extend(backend_diagnostics);
    (llvm.unwrap_or_default(), diagnostics)
}

/// Parse Gin source, transform it, lower to MLIR, and return the printed MLIR text
/// plus any codegen symptoms.
///
/// `debug_info` enables MLIR debug-info printing (source `loc(...)` attributes);
/// tests that assert on source locations must pass `true`.
pub fn codegen_to_mlir_text(
    source: &str,
    filename: &str,
    debug_info: bool,
) -> (String, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let typed = transform_file(ast, FileId(0));
    let context = NativeCompiler::create_context();

    let (module, symptoms) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &test_target(),
    );
    let flags = OperationPrintingFlags::new().print_generic_operation_form();
    let flags = if debug_info {
        flags.enable_debug_info(true, false)
    } else {
        flags
    };
    let module = module.unwrap_or_else(|| panic!("codegen should succeed: {symptoms:?}"));
    if std::env::var("DEBUG_CODEGEN_FUNCS").is_ok() {
        let debug_flags = OperationPrintingFlags::new().print_generic_operation_form();
        eprintln!(
            "{}",
            module
                .as_operation()
                .to_string_with_flags(debug_flags)
                .unwrap_or_default()
        );
    }
    let mlir_text = module
        .as_operation()
        .to_string_with_flags(flags)
        .expect("MLIR print");
    (mlir_text, symptoms)
}

#[allow(dead_code)]
pub fn codegen_assembly_to_mlir_text(source: &str, filename: &str) -> (String, Vec<Diagnostic>) {
    codegen_assembly_to_mlir_text_for_target(source, filename, test_target())
}

#[allow(dead_code)]
pub fn codegen_assembly_to_mlir_text_for_target(
    source: &str,
    filename: &str,
    target: flask::CompileTarget,
) -> (String, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let mut transform_context = TransformCtx::new();
    transform_context.compile_target = target.clone();
    let typed = transform(&ast, FileId(0), &transform_context);
    assert!(ast.parse_warnings.is_empty(), "{:?}", ast.parse_warnings);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}\n{:?}",
        typed.all_flaws()
    );
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &target,
    );
    let Some(module) = module else {
        return (String::new(), diagnostics);
    };
    let flags = OperationPrintingFlags::new().print_generic_operation_form();
    (
        module
            .as_operation()
            .to_string_with_flags(flags)
            .expect("MLIR print"),
        diagnostics,
    )
}

pub fn codegen_to_mlir_text_with_package_context(
    source: &str,
    filename: &str,
) -> (String, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let mut artifacts = typecheck::transform::transform_package_with_shared_context(
        vec![ast],
        typecheck::transform::PackageTransformOptions::FULL,
    );
    let typed = artifacts.typed_asts.remove(0);
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &test_target(),
    );
    let flags = OperationPrintingFlags::new().print_generic_operation_form();
    let mlir = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string_with_flags(flags)
        .expect("MLIR print");
    (mlir, diagnostics)
}

pub fn codegen_verifies(source: &str, filename: &str) -> (bool, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let typed = transform_file(ast, FileId(0));

    let registry = DialectRegistry::new();
    utility::register_all_dialects(&registry);
    let context = Context::new_with_registry(&registry, false);
    context.load_all_available_dialects();

    let (module, symptoms) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &test_target(),
    );
    let verified = module.is_some_and(|module| module.as_operation().verify());
    (verified, symptoms)
}

#[allow(dead_code)]
pub fn typed(source: &str) -> TypedFileAst {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{source}: {:?}",
        parsed.parse_warnings
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = test_fixtures::host_compile_target();
    let typed = transform(&parsed, FileId(0), &scope);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
    typed
}

#[allow(dead_code)]
pub fn assert_valid_source(source: &str) {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let typed = transform_file(parsed, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

/// Build a native engine for `source`, asserting the parse, typecheck, and codegen
/// stages are all clean. The target is the host triple the tests run on.
#[allow(dead_code)]
pub fn native_engine_for_source(source: &str, filename: &str, export: &str) -> NativeEngine {
    let parsed = parser::query::SourceParseExt::parse_source_full(source);
    assert!(
        parsed.symptoms.is_empty(),
        "{source}: {:?}",
        parsed.symptoms
    );
    let target = test_fixtures::host_compile_target();
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let typed = transform(&parsed.ast, FileId(0), &scope);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new(filename, source, &typed.span_table),
        &target,
    );
    assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
    NativeEngine::new_with_export_abi(module.unwrap(), export).unwrap()
}

#[allow(dead_code)]
pub fn llvm_function_body<'a>(llvm: &'a str, name: &str) -> &'a str {
    let definition = llvm
        .lines()
        .find(|line| line.starts_with("define ") && line.contains(&format!("@{name}(")))
        .unwrap();
    let start = llvm.find(definition).unwrap();
    llvm[start..].split_once("\n}").unwrap().0
}
