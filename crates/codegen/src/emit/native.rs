//! Native compilation: MLIR module → object file → linked executable.

// TODO: Add proper tail-call optimization. (unbounded tail calls in constant space).
// guarantee the same for self-recursive tail calls. recognize tail position, then use a
// jump instead of a call.
// maybe also greedy-shuffling to minimize argument-copying overhead in tail calls.

use super::Profile;
use super::toolchain;
use crate::build_module_with_context;
use ast::FileAst;
use diagnostic::codegen::CodegenSymptom;
use diagnostic::{Diagnostic, DiagnosticLike};
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
use span::SpanId;
use std::path::Path;
use std::process::Command;

/// Create a fully-initialized MLIR context with all dialects and LLVM translations.
fn create_native_context() -> Context {
    let context = Context::new();

    let registry = DialectRegistry::new();
    utility::register_all_dialects(&registry);
    context.append_dialect_registry(&registry);
    context.load_all_available_dialects();
    utility::register_all_llvm_translations(&context);

    context
}

/// Run MLIR optimization passes appropriate for the build profile.
///
/// Release: canonicalize (constant folding, dead code) + CSE (common subexpression elimination).
/// Debug: no-op.
fn optimize_mlir(
    context: &Context,
    module: &mut Module,
    profile: Profile,
    symptoms: &mut Vec<Diagnostic>,
) -> bool {
    if !matches!(profile, Profile::Release) {
        return true;
    }

    let pm = pass::PassManager::new(context);
    pm.add_pass(pass::transform::create_canonicalizer());
    pm.add_pass(pass::transform::create_cse());

    match pm.run(module) {
        Ok(()) => true,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("Optimization pass failed: {e}"),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            false
        }
    }
}

/// Lower a module in-place: SCF → CF, then everything → LLVM.
fn lower_to_llvm(context: &Context, module: &mut Module, symptoms: &mut Vec<Diagnostic>) -> bool {
    let pm = pass::PassManager::new(context);

    // SCF (structured control flow) must be lowered to CF first
    pm.add_pass(pass::conversion::create_scf_to_control_flow());
    // Then lower arith, func, cf, and remaining ops to LLVM
    pm.add_pass(pass::conversion::create_to_llvm());
    // Clean up any unrealized casts left over
    pm.add_pass(pass::conversion::create_reconcile_unrealized_casts());

    match pm.run(module) {
        Ok(()) => true,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("MLIR pass pipeline failed: {e}"),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            false
        }
    }
}

/// Fix `operandSegmentSizes` in MLIR text for `llvm.call` operations.
///
/// `OperationBuilder` can't set `operandSegmentSizes` as a computed property
/// in LLVM 21. This patches the serialized text to correct the values before
/// re-parsing.
fn fix_llvm_call_segments(mlir_text: &str) -> String {
    use std::fmt::Write;

    let mut result = String::with_capacity(mlir_text.len());
    let mut remaining = mlir_text;

    while let Some(call_start) = remaining.find("\"llvm.call\"(") {
        result.push_str(&remaining[..call_start]);
        let after_call = &remaining[call_start..];

        let open_paren = "\"llvm.call\"(".len();
        if let Some(close_paren) = after_call[open_paren..].find(')') {
            let operand_str = &after_call[open_paren..open_paren + close_paren];
            let num_operands = if operand_str.trim().is_empty() {
                0i32
            } else {
                operand_str.split(',').count() as i32
            };

            if let Some(seg_start) = after_call.find("operandSegmentSizes = array<i32:") {
                result.push_str(&after_call[..seg_start]);
                let after_seg = &after_call[seg_start..];
                if let Some(seg_end) = after_seg.find('>') {
                    // Segments are [normalOperands, op_bundle_operands]
                    let _ = write!(
                        result,
                        "operandSegmentSizes = array<i32: {num_operands}, 0>"
                    );
                    remaining = &after_call[seg_start + seg_end + 1..];
                    continue;
                }
            }
        }

        result.push_str(&after_call[..open_paren]);
        remaining = &after_call[open_paren..];
    }
    result.push_str(remaining);
    result
}

/// Compile pre-generated MLIR text to a native object file.
///
/// Pipeline: MLIR text → fix llvm.call → re-parse → optimize (Release)
/// → lower to LLVM dialect → mlir-translate → LLVM IR → cc -c → object file.
pub fn native_from_mlir(
    mlir_text: &str,
    obj_path: &Path,
    profile: Profile,
) -> (bool, Vec<Diagnostic>) {
    let mut symptoms = Vec::new();
    let context = create_native_context();

    let fixed_text = fix_llvm_call_segments(mlir_text);
    let mut module = match Module::parse(&context, &fixed_text) {
        Some(m) => m,
        None => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: "Failed to re-parse MLIR for native compilation".into(),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            return (false, symptoms);
        }
    };

    if !optimize_mlir(&context, &mut module, profile, &mut symptoms) {
        return (false, symptoms);
    }
    if !lower_to_llvm(&context, &mut module, &mut symptoms) {
        return (false, symptoms);
    }

    let lowered_mlir = module.as_operation().to_string();
    let Some(llvm_ir) = toolchain::mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
        return (false, symptoms);
    };
    let ok = toolchain::compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
    (ok, symptoms)
}

/// Compile a pre-built MLIR `Module` to a native object file.
///
/// This is the preferred codegen path when you already have a `Module` in hand.
/// It reuses the caller's context instead of creating a second one.
///
/// **Note:** Due to an LLVM 21 limitation where `OperationBuilder` cannot set
/// `operandSegmentSizes` for `llvm.call`, the module is briefly serialized to
/// text so `fix_llvm_call_segments` can patch it, then re-parsed into the same
/// context. This round-trip will be eliminated once the upstream API is fixed.
///
/// Pipeline: serialize → fix `llvm.call` segments → re-parse → optimize (Release)
/// → lower to LLVM dialect → `mlir-translate` → LLVM IR → `cc -c` → object file.
pub fn native_from_module(
    context: &Context,
    module: &Module,
    obj_path: &Path,
    profile: Profile,
) -> (bool, Vec<Diagnostic>) {
    let mut symptoms = Vec::new();
    let mlir_text = module.as_operation().to_string();

    let fixed_text = fix_llvm_call_segments(&mlir_text);

    let mut fixed_module = match Module::parse(context, &fixed_text) {
        Some(m) => m,
        None => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: "Failed to re-parse MLIR after fixing llvm.call segments".into(),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            return (false, symptoms);
        }
    };

    if !optimize_mlir(context, &mut fixed_module, profile, &mut symptoms) {
        return (false, symptoms);
    }
    if !lower_to_llvm(context, &mut fixed_module, &mut symptoms) {
        return (false, symptoms);
    }

    let lowered_mlir = fixed_module.as_operation().to_string();
    let Some(llvm_ir) = toolchain::mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
        return (false, symptoms);
    };
    let ok = toolchain::compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
    (ok, symptoms)
}

/// Used for `--emit mlir` and other cases where only the textual IR is needed.
pub fn build_module_text(
    ast: &mut FileAst,
    source: &str,
    filename: &str,
) -> (Option<String>, Vec<Diagnostic>) {
    let context = create_native_context();
    let (source_module, symptoms) =
        build_module_with_context(&context, ast, None, source, filename);
    let text = source_module.map(|m| m.as_operation().to_string());
    (text, symptoms)
}

pub fn build_module_text_from_typed(
    typed: &ast::typed::TypedFileAst,
    source: &str,
    filename: &str,
) -> (Option<String>, Vec<Diagnostic>) {
    let context = create_native_context();
    let module = crate::lower::build_module_from_typed_ast(&context, typed, source, filename);
    let text = module.map(|m| m.as_operation().to_string());
    (text, Vec::new())
}

pub fn compile_to_object(
    ast: &mut FileAst,
    obj_path: &Path,
    profile: Profile,
    source: &str,
    filename: &str,
) -> (bool, Vec<Diagnostic>) {
    let context = create_native_context();
    let (source_module, lower_symptoms) =
        build_module_with_context(&context, ast, None, source, filename);
    let mut symptoms = lower_symptoms;
    let Some(source_module) = source_module else {
        return (false, symptoms);
    };
    let (ok, more) = native_from_module(&context, &source_module, obj_path, profile);
    symptoms.extend(more);
    (ok, symptoms)
}

pub fn compile_to_object_from_typed(
    typed: &ast::typed::TypedFileAst,
    obj_path: &Path,
    profile: Profile,
    source: &str,
    filename: &str,
) -> (bool, Vec<Diagnostic>) {
    let context = create_native_context();
    // Re-parse the source to get a FileAst, then use the well-tested codegen path
    // with the typed AST for type resolution.
    let mut file_ast = parser::parse_from_str(source);
    let (module, symptoms) =
        build_module_with_context(&context, &mut file_ast, Some(typed), source, filename);
    let Some(module) = module else {
        return (false, symptoms);
    };
    let (ok, more) = native_from_module(&context, &module, obj_path, profile);
    let mut all = symptoms;
    all.extend(more);
    (ok, all)
}

pub fn link_executable(
    obj_path: &Path,
    output: &Path,
    target: Option<&str>,
) -> (bool, Vec<Diagnostic>) {
    let mut symptoms = Vec::new();

    let Some(cc) = toolchain::find_cc(&mut symptoms) else {
        return (false, symptoms);
    };

    let mut cmd = Command::new(&cc);
    cmd.arg("-o").arg(output).arg(obj_path);
    if let Some(triple) = target {
        cmd.arg(format!("--target={triple}"));
    }

    let result = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("Failed to run linker '{cc}': {e}"),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            return (false, symptoms);
        }
    };

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!(
                    "Linking failed (exit {}):\n{stderr}",
                    result.status.code().unwrap_or(-1)
                ),
            }
            .into_diagnostic(SpanId::INVALID),
        );
        return (false, symptoms);
    }

    (true, symptoms)
}
