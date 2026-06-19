//! Native compilation: MLIR module → object file → linked executable.

// TODO: Add proper tail-call optimization. (unbounded tail calls in constant space).
// guarantee the same for self-recursive tail calls. recognize tail position, then use a
// jump instead of a call.
// maybe also greedy-shuffling to minimize argument-copying overhead in tail calls.

use super::Profile;
use diagnostic::Diagnostic;
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
use std::path::Path;
use std::process::Command;

/// A native compiler that owns an MLIR context and manages the compilation pipeline.
#[derive(Default)]
pub struct NativeCompiler {
    context: Context,
}

impl NativeCompiler {
    /// Compile pre-generated MLIR text to a native object file.
    ///
    /// Pipeline: MLIR text → fix llvm.call → re-parse → optimize (Release)
    /// → lower to LLVM dialect → mlir-translate → LLVM IR → cc -c → object file.
    pub fn native_from_mlir(
        &self,
        mlir_text: &str,
        obj_path: &Path,
        profile: Profile,
    ) -> (bool, Vec<Diagnostic>) {
        let mut symptoms = Vec::new();

        let fixed_text = Self::fix_llvm_call_segments(mlir_text);
        let mut module = match Module::parse(&self.context, &fixed_text) {
            Some(m) => m,
            None => {
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        "Failed to re-parse MLIR for native compilation",
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return (false, symptoms);
            }
        };

        if !self.optimize(&mut module, profile, &mut symptoms) {
            return (false, symptoms);
        }
        if !self.lower_to_llvm(&mut module, &mut symptoms) {
            return (false, symptoms);
        }

        let lowered_mlir = module.as_operation().to_string();
        let Some(llvm_ir) = Self::mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
            return (false, symptoms);
        };
        let ok = Self::compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
        (ok, symptoms)
    }

    /// Compile a pre-built MLIR `Module` to a native object file.
    ///
    /// This is the preferred codegen path when you already have a `Module` in hand.
    ///
    /// **Note:** Due to an LLVM 21 limitation where `OperationBuilder` cannot set
    /// `operandSegmentSizes` for `llvm.call`, the module is briefly serialized to
    /// text so `fix_llvm_call_segments` can patch it, then re-parsed into the same
    /// context. This round-trip will be eliminated once the upstream API is fixed.
    ///
    /// Pipeline: serialize → fix `llvm.call` segments → re-parse → optimize (Release)
    /// → lower to LLVM dialect → `mlir-translate` → LLVM IR → `cc -c` → object file.
    pub fn native_from_module(
        &self,
        module: &Module,
        obj_path: &Path,
        profile: Profile,
    ) -> (bool, Vec<Diagnostic>) {
        let mut symptoms = Vec::new();
        let mlir_text = module.as_operation().to_string();

        let fixed_text = Self::fix_llvm_call_segments(&mlir_text);

        let mut fixed_module = match Module::parse(&self.context, &fixed_text) {
            Some(m) => m,
            None => {
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        "Failed to re-parse MLIR after fixing llvm.call segments",
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return (false, symptoms);
            }
        };

        if !self.optimize(&mut fixed_module, profile, &mut symptoms) {
            return (false, symptoms);
        }
        if !self.lower_to_llvm(&mut fixed_module, &mut symptoms) {
            return (false, symptoms);
        }

        let lowered_mlir = fixed_module.as_operation().to_string();
        let Some(llvm_ir) = Self::mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
            return (false, symptoms);
        };
        let ok = Self::compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
        (ok, symptoms)
    }

    /// Run MLIR optimization passes appropriate for the build profile.
    ///
    /// Release: canonicalize (constant folding, dead code) + CSE (common subexpression elimination).
    /// Debug: no-op.
    fn optimize(
        &self,
        module: &mut Module,
        profile: Profile,
        symptoms: &mut Vec<Diagnostic>,
    ) -> bool {
        if !matches!(profile, Profile::Release) {
            return true;
        }

        let pm = pass::PassManager::new(&self.context);
        pm.add_pass(pass::transform::create_canonicalizer());
        pm.add_pass(pass::transform::create_cse());

        match pm.run(module) {
            Ok(()) => true,
            Err(e) => {
                symptoms.push(
                    Diagnostic::new("codegen-internal", format!("Optimization pass failed: {e}"))
                        .with_help("an internal compiler error occurred")
                        .at_span(diagnostic::Span::new(0, 0)),
                );
                false
            }
        }
    }

    /// Lower a module in-place: SCF → CF, then everything → LLVM.
    fn lower_to_llvm(&self, module: &mut Module, symptoms: &mut Vec<Diagnostic>) -> bool {
        let pm = pass::PassManager::new(&self.context);

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
                    Diagnostic::new(
                        "codegen-internal",
                        format!("MLIR pass pipeline failed: {e}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                false
            }
        }
    }

    /// Create a fully-initialized MLIR context with all dialects and LLVM translations.
    pub fn create_context() -> Context {
        let context = Context::new();

        let registry = DialectRegistry::new();
        utility::register_all_dialects(&registry);
        context.append_dialect_registry(&registry);
        context.load_all_available_dialects();
        utility::register_all_llvm_translations(&context);

        context
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

    /// Link an object file into an executable.
    pub fn link_executable(
        obj_path: &Path,
        output: &Path,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>) {
        let mut symptoms = Vec::new();

        let Some(cc) = Self::find_cc(&mut symptoms) else {
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
                    Diagnostic::new(
                        "codegen-internal",
                        format!("Failed to run linker '{cc}': {e}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return (false, symptoms);
            }
        };

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            symptoms.push(
                Diagnostic::new(
                    "codegen-internal",
                    format!(
                        "Linking failed (exit {}):\n{stderr}",
                        result.status.code().unwrap_or(-1)
                    ),
                )
                .with_help("an internal compiler error occurred")
                .at_span(diagnostic::Span::new(0, 0)),
            );
            return (false, symptoms);
        }

        (true, symptoms)
    }
}
