//! Native compilation: MLIR module → object file → linked executable.

// TODO: Add proper tail-call optimization. (unbounded tail calls in constant space).
// guarantee the same for self-recursive tail calls. recognize tail position, then use a
// jump instead of a call.
// maybe also greedy-shuffling to minimize argument-copying overhead in tail calls.

use crate::build_module_with_context;
use ast::FileAst;
use diagnostic::codegen::CodegenSymptom;
use diagnostic::{Symptom, SymptomLike};
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
use span::SpanId;
use std::path::Path;
use std::process::Command;
use typeck::TyEnv;

/// Build profile for optimization levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Debug,
    Release,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

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
    symptoms: &mut Vec<Symptom>,
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
                .into_symptom(SpanId::INVALID),
            );
            false
        }
    }
}

/// Lower a module in-place: SCF → CF, then everything → LLVM.
fn lower_to_llvm(context: &Context, module: &mut Module, symptoms: &mut Vec<Symptom>) -> bool {
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
                .into_symptom(SpanId::INVALID),
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
) -> (bool, Vec<Symptom>) {
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
                .into_symptom(SpanId::INVALID),
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
    let Some(llvm_ir) = mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
        return (false, symptoms);
    };
    let ok = compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
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
) -> (bool, Vec<Symptom>) {
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
                .into_symptom(SpanId::INVALID),
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
    let Some(llvm_ir) = mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
        return (false, symptoms);
    };
    let ok = compile_llvm_ir_to_object(&llvm_ir, obj_path, profile, &mut symptoms);
    (ok, symptoms)
}

/// Build an MLIR module from the AST and return the MLIR text.
///
/// Used for `--emit mlir` and other cases where only the textual IR is needed.
pub fn build_module_text(
    ast: &FileAst,
    source: &str,
    filename: &str,
    ty_env: &TyEnv,
) -> (Option<String>, Vec<Symptom>) {
    let context = create_native_context();

    let (source_module, symptoms) =
        build_module_with_context(&context, ast, source, filename, ty_env);

    let text = source_module.map(|m| m.as_operation().to_string());
    (text, symptoms)
}

/// Build an MLIR module from the AST, lower to LLVM, and compile to an object file.
pub fn compile_to_object(
    ast: &FileAst,
    obj_path: &Path,
    profile: Profile,
    source: &str,
    filename: &str,
    ty_env: &TyEnv,
) -> (bool, Vec<Symptom>) {
    let context = create_native_context();

    let (source_module, lower_symptoms) =
        build_module_with_context(&context, ast, source, filename, ty_env);

    let mut symptoms = lower_symptoms;

    let Some(source_module) = source_module else {
        return (false, symptoms);
    };

    let (ok, more) = native_from_module(&context, &source_module, obj_path, profile);
    symptoms.extend(more);
    (ok, symptoms)
}

/// Translate LLVM-dialect MLIR text to LLVM IR using `mlir-translate`.
fn mlir_to_llvm_ir(mlir_text: &str, symptoms: &mut Vec<Symptom>) -> Option<String> {
    let mlir_translate = find_tool("mlir-translate", symptoms)?;

    let mut cmd = Command::new(&mlir_translate);
    cmd.arg("--mlir-to-llvmir");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("Failed to run mlir-translate: {e}"),
                }
                .into_symptom(SpanId::INVALID),
            );
            return None;
        }
    };

    use std::io::Write;
    if let Err(e) = child.stdin.take().unwrap().write_all(mlir_text.as_bytes()) {
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!("Failed to write to mlir-translate stdin: {e}"),
            }
            .into_symptom(SpanId::INVALID),
        );
        return None;
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("mlir-translate failed: {e}"),
                }
                .into_symptom(SpanId::INVALID),
            );
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!(
                    "mlir-translate failed (exit {}):\n{stderr}",
                    output.status.code().unwrap_or(-1)
                ),
            }
            .into_symptom(SpanId::INVALID),
        );
        return None;
    }

    match String::from_utf8(output.stdout) {
        Ok(s) => Some(s),
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("mlir-translate output is not UTF-8: {e}"),
                }
                .into_symptom(SpanId::INVALID),
            );
            None
        }
    }
}

/// Compile LLVM IR text to an object file using `cc -c`.
fn compile_llvm_ir_to_object(
    llvm_ir: &str,
    obj_path: &Path,
    profile: Profile,
    symptoms: &mut Vec<Symptom>,
) -> bool {
    let ll_path = obj_path.with_extension("ll");

    if let Err(e) = std::fs::write(&ll_path, llvm_ir) {
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!("Failed to write LLVM IR: {e}"),
            }
            .into_symptom(SpanId::INVALID),
        );
        return false;
    }

    let Some(cc) = find_cc(symptoms) else {
        let _ = std::fs::remove_file(&ll_path);
        return false;
    };

    let opt_flag = match profile {
        Profile::Release => "-O2",
        Profile::Debug => "-O0",
    };

    let result = match Command::new(&cc)
        .arg("-c")
        .arg(opt_flag)
        .arg(&ll_path)
        .arg("-o")
        .arg(obj_path)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: format!("Failed to run '{cc}': {e}"),
                }
                .into_symptom(SpanId::INVALID),
            );
            let _ = std::fs::remove_file(&ll_path);
            return false;
        }
    };

    let _ = std::fs::remove_file(&ll_path);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!(
                    "Compiling LLVM IR failed (exit {}):\n{stderr}",
                    result.status.code().unwrap_or(-1)
                ),
            }
            .into_symptom(SpanId::INVALID),
        );
        return false;
    }

    true
}

/// Link an object file into an executable using the system C compiler.
pub fn link_executable(
    obj_path: &Path,
    output: &Path,
    target: Option<&str>,
) -> (bool, Vec<Symptom>) {
    let mut symptoms = Vec::new();

    let Some(cc) = find_cc(&mut symptoms) else {
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
                .into_symptom(SpanId::INVALID),
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
            .into_symptom(SpanId::INVALID),
        );
        return (false, symptoms);
    }

    (true, symptoms)
}

/// Find `mlir-translate` or similar LLVM/MLIR tool.
fn find_tool(name: &str, symptoms: &mut Vec<Symptom>) -> Option<String> {
    // Check PATH first
    if let Ok(output) = Command::new(name).arg("--help").output()
        && output.status.success()
    {
        return Some(name.to_string());
    }

    // Check common Homebrew LLVM paths
    for prefix in &["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"] {
        let path = format!("{prefix}/{name}");
        if Path::new(&path).exists() {
            return Some(path);
        }
    }

    symptoms.push(
        CodegenSymptom::Internal {
            message: format!("'{name}' not found. Install LLVM or add it to PATH."),
        }
        .into_symptom(SpanId::INVALID),
    );
    None
}

/// Find a C compiler: check $CC, then prefer Homebrew LLVM clang (same version
/// as mlir-translate), then fall back to `cc`.
fn find_cc(symptoms: &mut Vec<Symptom>) -> Option<String> {
    if let Ok(cc) = std::env::var("CC")
        && !cc.is_empty()
    {
        return Some(cc);
    }

    // Prefer the Homebrew LLVM clang to ensure IR attribute compatibility.
    // Check both generic and versioned paths (llvm, llvm@21, llvm@20, etc.).
    let brew_prefixes = ["/opt/homebrew/opt", "/usr/local/opt"];
    let llvm_variants = ["llvm", "llvm@21", "llvm@20", "llvm@19", "llvm@18"];
    for prefix in &brew_prefixes {
        for variant in &llvm_variants {
            let candidate = format!("{prefix}/{variant}/bin/clang");
            if Path::new(&candidate).exists() {
                return Some(candidate);
            }
        }
    }

    // Fall back to system cc.
    match Command::new("cc").arg("--version").output() {
        Ok(output) if output.status.success() => Some("cc".into()),
        _ => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: "No C compiler found. Install LLVM or set $CC.".into(),
                }
                .into_symptom(SpanId::INVALID),
            );
            None
        }
    }
}
