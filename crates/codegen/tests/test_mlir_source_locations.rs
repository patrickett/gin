//! Integration tests: MLIR ops should carry `FileLineCol` locations back to the `.gin` source.

use codegen::build_module_with_context;
use diagnostic::Diagnostic;
use melior::{
    Context,
    ir::operation::{OperationLike, OperationPrintingFlags},
};
use parser::parse_from_str;
use typeck::TyEnv;

fn codegen_to_mlir_text(source: &str, filename: &str) -> (String, Vec<Diagnostic>) {
    let ast = parse_from_str(source);
    let ty_env = TyEnv::from_file_ast(&ast);

    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    let (module, symptoms) = build_module_with_context(&context, &ast, source, filename, &ty_env);
    let mlir_text = module
        .expect("codegen should succeed")
        .as_operation()
        .to_string_with_flags(
            OperationPrintingFlags::new().enable_debug_info(true, false),
        )
        .expect("MLIR print");
    (mlir_text, symptoms)
}

/// Zero-arg callee used so the emitted IR contains a single obvious `func.call`.
const TWO_FN_PROGRAM: &str = "\
noop() Unit:\n\
return\n\
\n\
main:\n\
    noop()\n\
return\n\
";

#[test]
fn func_call_carries_gin_filename_and_call_line_in_loc() {
    let filename = "mlir_source_loc_tracking.gin";
    let (mlir_text, symptoms) = codegen_to_mlir_text(TWO_FN_PROGRAM, filename);
    assert!(
        symptoms.is_empty(),
        "expected no codegen symptoms: {symptoms:?}"
    );
    assert!(
        mlir_text.contains("callee = @noop"),
        "expected a call to noop; got:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains(&format!(r#"loc("{filename}""#)),
        "expected MLIR loc() to name the Gin file {filename:?}; got:\n{mlir_text}"
    );
    assert!(
        mlir_text.contains(&format!(r#"{filename}":5:"#)),
        "noop() call is on source line 5; expected loc(...:5:...) with that file; got:\n{mlir_text}"
    );
}
