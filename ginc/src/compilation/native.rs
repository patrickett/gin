//! Native compilation: MLIR module → object file → linked executable.

use crate::args::Profile;
use crate::ast::FileAst;
use crate::codegen::build_module_with_context;
use crate::diagnostic::codegen::CodegenSymptom;
use chumsky::span::{SimpleSpan, Span};
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
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
) -> Result<(), CodegenSymptom> {
    if !matches!(profile, Profile::Release) {
        return Ok(());
    }

    let pm = pass::PassManager::new(context);
    pm.add_pass(pass::transform::create_canonicalizer());
    pm.add_pass(pass::transform::create_cse());

    pm.run(module).map_err(|e| CodegenSymptom::Internal {
        message: format!("Optimization pass failed: {e}"),
        span: SimpleSpan::new((), 0..0),
    })
}

/// Lower a module in-place: SCF → CF, then everything → LLVM.
fn lower_to_llvm(context: &Context, module: &mut Module) -> Result<(), CodegenSymptom> {
    let pm = pass::PassManager::new(context);

    // SCF (structured control flow) must be lowered to CF first
    pm.add_pass(pass::conversion::create_scf_to_control_flow());
    // Then lower arith, func, cf, and remaining ops to LLVM
    pm.add_pass(pass::conversion::create_to_llvm());
    // Clean up any unrealized casts left over
    pm.add_pass(pass::conversion::create_reconcile_unrealized_casts());

    pm.run(module).map_err(|e| CodegenSymptom::Internal {
        message: format!("MLIR pass pipeline failed: {e}"),
        span: SimpleSpan::new((), 0..0),
    })
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
) -> Result<(), CodegenSymptom> {
    let context = create_native_context();

    let fixed_text = fix_llvm_call_segments(mlir_text);
    let mut module =
        Module::parse(&context, &fixed_text).ok_or_else(|| CodegenSymptom::Internal {
            message: "Failed to re-parse MLIR for native compilation".into(),
            span: SimpleSpan::new((), 0..0),
        })?;

    optimize_mlir(&context, &mut module, profile)?;
    lower_to_llvm(&context, &mut module)?;

    let lowered_mlir = module.as_operation().to_string();
    let llvm_ir = mlir_to_llvm_ir(&lowered_mlir)?;
    compile_llvm_ir_to_object(&llvm_ir, obj_path, profile)?;

    Ok(())
}

/// Build an MLIR module from the AST, lower to LLVM, and compile to an object file.
pub fn compile_to_object(
    ast: &FileAst,
    obj_path: &Path,
    profile: Profile,
    source: &str,
    filename: &str,
) -> Result<(), CodegenSymptom> {
    let context = create_native_context();

    let (source_module, symptoms) = build_module_with_context(&context, ast, source, filename);
    let source_module = match source_module {
        Some(m) => m,
        None => {
            // Return the first symptom as the error, or a generic one
            return Err(symptoms
                .into_iter()
                .next()
                .unwrap_or(CodegenSymptom::Internal {
                    message: "Codegen failed with no specific symptom".into(),
                    span: SimpleSpan::new((), 0..0),
                }));
        }
    };
    let mlir_text = source_module.as_operation().to_string();
    drop(source_module);

    native_from_mlir(&mlir_text, obj_path, profile)
}

/// Translate LLVM-dialect MLIR text to LLVM IR using `mlir-translate`.
fn mlir_to_llvm_ir(mlir_text: &str) -> Result<String, CodegenSymptom> {
    let mlir_translate = find_tool("mlir-translate")?;

    let mut cmd = Command::new(&mlir_translate);
    cmd.arg("--mlir-to-llvmir");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| CodegenSymptom::Internal {
        message: format!("Failed to run mlir-translate: {e}"),
        span: SimpleSpan::new((), 0..0),
    })?;

    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(mlir_text.as_bytes())
        .map_err(|e| CodegenSymptom::Internal {
            message: format!("Failed to write to mlir-translate stdin: {e}"),
            span: SimpleSpan::new((), 0..0),
        })?;

    let output = child
        .wait_with_output()
        .map_err(|e| CodegenSymptom::Internal {
            message: format!("mlir-translate failed: {e}"),
            span: SimpleSpan::new((), 0..0),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CodegenSymptom::Internal {
            message: format!(
                "mlir-translate failed (exit {}):\n{stderr}",
                output.status.code().unwrap_or(-1)
            ),
            span: SimpleSpan::new((), 0..0),
        });
    }

    String::from_utf8(output.stdout).map_err(|e| CodegenSymptom::Internal {
        message: format!("mlir-translate output is not UTF-8: {e}"),
        span: SimpleSpan::new((), 0..0),
    })
}

/// Compile LLVM IR text to an object file using `cc -c`.
fn compile_llvm_ir_to_object(
    llvm_ir: &str,
    obj_path: &Path,
    profile: Profile,
) -> Result<(), CodegenSymptom> {
    let ll_path = obj_path.with_extension("ll");

    std::fs::write(&ll_path, llvm_ir).map_err(|e| CodegenSymptom::Internal {
        message: format!("Failed to write LLVM IR: {e}"),
        span: SimpleSpan::new((), 0..0),
    })?;

    let cc = find_cc()?;
    let opt_flag = match profile {
        Profile::Release => "-O2",
        Profile::Debug => "-O0",
    };
    let result = Command::new(&cc)
        .arg("-c")
        .arg(opt_flag)
        .arg(&ll_path)
        .arg("-o")
        .arg(obj_path)
        .output()
        .map_err(|e| CodegenSymptom::Internal {
            message: format!("Failed to run '{cc}': {e}"),
            span: SimpleSpan::new((), 0..0),
        })?;

    // Clean up the .ll file regardless of success
    let _ = std::fs::remove_file(&ll_path);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(CodegenSymptom::Internal {
            message: format!(
                "Compiling LLVM IR failed (exit {}):\n{stderr}",
                result.status.code().unwrap_or(-1)
            ),
            span: SimpleSpan::new((), 0..0),
        });
    }

    Ok(())
}

/// Path to gin_core/runtime.c, resolved at compile time relative to this crate.
const GIN_CORE_RUNTIME_C: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../packages/gin_core/runtime.c"
);

/// Link an object file into an executable using the system C compiler.
/// Automatically compiles and links gin_core/runtime.c if it exists.
pub fn link_executable(
    obj_path: &Path,
    output: &Path,
    target: Option<&str>,
) -> Result<(), CodegenSymptom> {
    let cc = find_cc()?;

    let runtime_c = Path::new(GIN_CORE_RUNTIME_C);
    let runtime_obj = output.with_extension("runtime.o");
    let has_runtime = if runtime_c.exists() {
        let mut compile = Command::new(&cc);
        compile
            .arg("-c")
            .arg("-O2")
            .arg(runtime_c)
            .arg("-o")
            .arg(&runtime_obj);
        if let Some(triple) = target {
            compile.arg(format!("--target={triple}"));
        }
        let out = compile.output().map_err(|e| CodegenSymptom::Internal {
            message: format!("Failed to compile gin_core runtime: {e}"),
            span: SimpleSpan::new((), 0..0),
        })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(CodegenSymptom::Internal {
                message: format!("gin_core runtime compile failed:\n{stderr}"),
                span: SimpleSpan::new((), 0..0),
            });
        }
        true
    } else {
        false
    };

    let mut cmd = Command::new(&cc);
    cmd.arg("-o").arg(output).arg(obj_path);
    if has_runtime {
        cmd.arg(&runtime_obj);
    }
    if let Some(triple) = target {
        cmd.arg(format!("--target={triple}"));
    }

    let result = cmd.output().map_err(|e| CodegenSymptom::Internal {
        message: format!("Failed to run linker '{cc}': {e}"),
        span: SimpleSpan::new((), 0..0),
    })?;

    if has_runtime {
        let _ = std::fs::remove_file(&runtime_obj);
    }

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(CodegenSymptom::Internal {
            message: format!(
                "Linking failed (exit {}):\n{stderr}",
                result.status.code().unwrap_or(-1)
            ),
            span: SimpleSpan::new((), 0..0),
        });
    }

    Ok(())
}

/// Find `mlir-translate` or similar LLVM/MLIR tool.
fn find_tool(name: &str) -> Result<String, CodegenSymptom> {
    // Check PATH first
    if let Ok(output) = Command::new(name).arg("--help").output()
        && output.status.success()
    {
        return Ok(name.to_string());
    }

    // Check common Homebrew LLVM paths
    for prefix in &["/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"] {
        let path = format!("{prefix}/{name}");
        if Path::new(&path).exists() {
            return Ok(path);
        }
    }

    Err(CodegenSymptom::Internal {
        message: format!("'{name}' not found. Install LLVM or add it to PATH."),
        span: SimpleSpan::new((), 0..0),
    })
}

/// Find a C compiler: check $CC, then prefer Homebrew LLVM clang (same version
/// as mlir-translate), then fall back to `cc`.
fn find_cc() -> Result<String, CodegenSymptom> {
    if let Ok(cc) = std::env::var("CC")
        && !cc.is_empty()
    {
        return Ok(cc);
    }

    // Prefer the Homebrew LLVM clang to ensure IR attribute compatibility.
    // Check both generic and versioned paths (llvm, llvm@21, llvm@20, etc.).
    let brew_prefixes = ["/opt/homebrew/opt", "/usr/local/opt"];
    let llvm_variants = ["llvm", "llvm@21", "llvm@20", "llvm@19", "llvm@18"];
    for prefix in &brew_prefixes {
        for variant in &llvm_variants {
            let candidate = format!("{prefix}/{variant}/bin/clang");
            if Path::new(&candidate).exists() {
                return Ok(candidate);
            }
        }
    }

    // Fall back to system cc.
    match Command::new("cc").arg("--version").output() {
        Ok(output) if output.status.success() => Ok("cc".into()),
        _ => Err(CodegenSymptom::Internal {
            message: "No C compiler found. Install LLVM or set $CC.".into(),
            span: SimpleSpan::new((), 0..0),
        }),
    }
}
