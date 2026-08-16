use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler};
use diagnostic::Diagnostic;
use melior::Context;
use melior::dialect::DialectRegistry;
use melior::ir::operation::{OperationLike, OperationPrintingFlags};
use melior::utility;
use parser::parse_from_str;
use typecheck::FileId;
use typecheck::transform::transform_file;

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
    let module = module.expect("codegen should succeed");
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
