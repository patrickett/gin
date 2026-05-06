//! External tool discovery and invocation.

use diagnostic::codegen::CodegenSymptom;
use diagnostic::{Diagnostic, DiagnosticLike};
use span::SpanId;
use std::path::Path;
use std::process::Command;

use super::Profile;

/// Find an LLVM/MLIR tool on PATH or in common Homebrew locations.
pub(crate) fn find_tool(name: &str, symptoms: &mut Vec<Diagnostic>) -> Option<String> {
    if let Ok(output) = Command::new(name).arg("--help").output()
        && output.status.success()
    {
        return Some(name.to_string());
    }

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
        .into_diagnostic(SpanId::INVALID),
    );
    None
}

/// Find a C compiler: check `$CC`, then Homebrew LLVM clang, then fall back to `cc`.
pub(crate) fn find_cc(symptoms: &mut Vec<Diagnostic>) -> Option<String> {
    if let Ok(cc) = std::env::var("CC")
        && !cc.is_empty()
    {
        return Some(cc);
    }

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

    match Command::new("cc").arg("--version").output() {
        Ok(output) if output.status.success() => Some("cc".into()),
        _ => {
            symptoms.push(
                CodegenSymptom::Internal {
                    message: "No C compiler found. Install LLVM or set $CC.".into(),
                }
                .into_diagnostic(SpanId::INVALID),
            );
            None
        }
    }
}

/// Translate LLVM-dialect MLIR text to LLVM IR using `mlir-translate`.
pub(crate) fn mlir_to_llvm_ir(mlir_text: &str, symptoms: &mut Vec<Diagnostic>) -> Option<String> {
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
                .into_diagnostic(SpanId::INVALID),
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
            .into_diagnostic(SpanId::INVALID),
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
                .into_diagnostic(SpanId::INVALID),
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
            .into_diagnostic(SpanId::INVALID),
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
                .into_diagnostic(SpanId::INVALID),
            );
            None
        }
    }
}

/// Compile LLVM IR text to an object file using `cc -c`.
pub(crate) fn compile_llvm_ir_to_object(
    llvm_ir: &str,
    obj_path: &Path,
    profile: Profile,
    symptoms: &mut Vec<Diagnostic>,
) -> bool {
    let ll_path = obj_path.with_extension("ll");

    if let Err(e) = std::fs::write(&ll_path, llvm_ir) {
        symptoms.push(
            CodegenSymptom::Internal {
                message: format!("Failed to write LLVM IR: {e}"),
            }
            .into_diagnostic(SpanId::INVALID),
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
                .into_diagnostic(SpanId::INVALID),
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
            .into_diagnostic(SpanId::INVALID),
        );
        return false;
    }

    true
}
