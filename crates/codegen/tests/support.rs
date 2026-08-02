use codegen::CodegenContext;
use diagnostic::Diagnostic;
use melior::Context;
use melior::ir::operation::{OperationLike, OperationPrintingFlags};
use parser::parse_from_str;
use typecheck::FileId;
use typecheck::transform::transform_file;

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

    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    let (module, symptoms) =
        CodegenContext::build_module_from_typed_ast(&context, &typed, source, filename, None);
    let mlir_text = if debug_info {
        module
            .expect("codegen should succeed")
            .as_operation()
            .to_string_with_flags(OperationPrintingFlags::new().enable_debug_info(true, false))
            .expect("MLIR print")
    } else {
        module
            .expect("codegen should succeed")
            .as_operation()
            .to_string()
    };
    (mlir_text, symptoms)
}
