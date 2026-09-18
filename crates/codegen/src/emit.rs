//! Native compilation: MLIR module → object file → linked executable.

// TODO: Add proper tail-call optimization. (unbounded tail calls in constant space).
// guarantee the same for self-recursive tail calls. recognize tail position, then use a
// jump instead of a call.
// maybe also greedy-shuffling to minimize argument-copying overhead in tail calls.

use diagnostic::Diagnostic;
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
use std::io::Write;
use std::path::Path;
use std::process::Command;
use strum::Display;

/// Build profile for optimization levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
pub enum Profile {
    #[default]
    #[strum(to_string = "debug")]
    Debug,
    #[strum(to_string = "release")]
    Release,
}

const GIN_RUNTIME_C: &str = r#"
#include <stdio.h>

long long runtime_read_int(void) {
    long long value = 0;
    if (scanf("%lld", &value) != 1) {
        return 0;
    }
    return value;
}

long long runtime_print_int(long long value) {
    printf("%lld\n", value);
    return 0;
}

long long runtime_echo(void) {
    int value;
    while ((value = getchar()) != EOF) {
        if (putchar(value) == EOF) {
            return 1;
        }
    }
    return 0;
}
"#;

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

impl std::str::FromStr for Profile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "debug" => Ok(Self::Debug),
            "release" => Ok(Self::Release),
            _ => Err(format!(
                "unknown build profile '{s}', expected 'debug' or 'release'"
            )),
        }
    }
}

/// A native compiler that owns an MLIR context and manages the compilation pipeline.
pub struct NativeCompiler {
    context: Context,
}

/// Backend timings for the MLIR->object pipeline.
#[derive(Default, Debug, Clone, Copy)]
pub struct NativeCodegenTimings {
    pub optimization: std::time::Duration,
    pub lowering: std::time::Duration,
    pub object_emission: std::time::Duration,
}

impl Default for NativeCompiler {
    fn default() -> Self {
        Self {
            context: Self::create_context(),
        }
    }
}

impl NativeCompiler {
    pub(crate) fn context(&self) -> &Context {
        &self.context
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
    pub fn llvm_ir_from_module(
        &self,
        module: &Module,
        profile: Profile,
        fast: bool,
    ) -> (Option<String>, Vec<Diagnostic>) {
        let mut symptoms = Vec::new();
        let mut module = match self.reparse_for_backend(module) {
            Ok(module) => module,
            Err(diagnostic) => {
                symptoms.push(*diagnostic);
                return (None, symptoms);
            }
        };
        if !self.optimize(&mut module, profile, fast, &mut symptoms)
            || !self.lower_to_llvm(&mut module, &mut symptoms)
        {
            return (None, symptoms);
        }
        let lowered = module.as_operation().to_string();
        let llvm = Self::mlir_to_llvm_ir(&lowered, &mut symptoms);
        (llvm, symptoms)
    }

    /// Compile a pre-built MLIR `Module` to a native object file and return phase timings.
    pub fn native_from_module_with_timings(
        &self,
        module: &Module,
        obj_path: &Path,
        profile: Profile,
        fast: bool,
    ) -> (bool, Vec<Diagnostic>, NativeCodegenTimings) {
        self.native_from_module_with_timings_for_target(module, obj_path, profile, fast, None)
    }

    pub fn native_from_module_with_timings_for_target(
        &self,
        module: &Module,
        obj_path: &Path,
        profile: Profile,
        fast: bool,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>, NativeCodegenTimings) {
        let mut symptoms = Vec::new();
        let mut timings = NativeCodegenTimings::default();
        let mut fixed_module = match self.reparse_for_backend(module) {
            Ok(module) => module,
            Err(diagnostic) => {
                symptoms.push(*diagnostic);
                return (false, symptoms, timings);
            }
        };

        let start = std::time::Instant::now();
        if !self.optimize(&mut fixed_module, profile, fast, &mut symptoms) {
            timings.optimization = start.elapsed();
            return (false, symptoms, timings);
        }
        timings.optimization = start.elapsed();

        let start = std::time::Instant::now();
        if !self.lower_to_llvm(&mut fixed_module, &mut symptoms) {
            timings.lowering = start.elapsed();
            return (false, symptoms, timings);
        }
        timings.lowering = start.elapsed();

        let start = std::time::Instant::now();
        let lowered_mlir = fixed_module.as_operation().to_string();
        let Some(llvm_ir) = Self::mlir_to_llvm_ir(&lowered_mlir, &mut symptoms) else {
            timings.lowering += start.elapsed();
            return (false, symptoms, timings);
        };
        timings.lowering += start.elapsed();

        let start = std::time::Instant::now();
        let ok = Self::compile_llvm_ir_to_object(
            &llvm_ir,
            obj_path,
            profile,
            fast,
            target,
            &mut symptoms,
        );
        timings.object_emission = start.elapsed();
        (ok, symptoms, timings)
    }

    /// Run MLIR optimization passes appropriate for the build profile.
    ///
    /// Release: canonicalize (constant folding, dead code) + CSE (common subexpression elimination).
    /// Debug: no-op.
    fn optimize(
        &self,
        module: &mut Module,
        profile: Profile,
        fast: bool,
        symptoms: &mut Vec<Diagnostic>,
    ) -> bool {
        if !matches!(profile, Profile::Release) || fast {
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
    pub(crate) fn lower_to_llvm(
        &self,
        module: &mut Module,
        symptoms: &mut Vec<Diagnostic>,
    ) -> bool {
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

    pub(crate) fn reparse_for_backend<'a>(
        &'a self,
        module: &Module,
    ) -> Result<Module<'a>, Box<Diagnostic>> {
        let mlir_text = module.as_operation().to_string();
        let fixed_text = Self::fix_llvm_call_segments(&mlir_text);

        Module::parse(&self.context, &fixed_text).ok_or_else(|| {
            Box::new(
                Diagnostic::new(
                    "codegen-internal",
                    "Failed to re-parse MLIR after fixing llvm.call segments",
                )
                .with_help("an internal compiler error occurred")
                .at_span(diagnostic::Span::new(0, 0)),
            )
        })
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
        let Some(needs_runtime) = Self::needs_legacy_runtime(obj_path, &mut symptoms) else {
            return (false, symptoms);
        };

        let runtime_object = output.with_extension(format!(
            "gin-runtime-{}-{}.o",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| duration.as_nanos())
        ));
        if needs_runtime {
            let mut runtime_compile = Command::new(&cc);
            runtime_compile
                .arg("-x")
                .arg("c")
                .arg("-c")
                .arg("-o")
                .arg(&runtime_object)
                .arg("-");
            if let Some(triple) = target {
                runtime_compile.arg(format!("--target={triple}"));
            }
            let mut runtime_compile =
                match runtime_compile.stdin(std::process::Stdio::piped()).spawn() {
                    Ok(child) => child,
                    Err(error) => {
                        symptoms.push(
                            Diagnostic::new(
                                "codegen-internal",
                                format!("Failed to run runtime compiler '{cc}': {error}"),
                            )
                            .with_help("an internal compiler error occurred")
                            .at_span(diagnostic::Span::new(0, 0)),
                        );
                        return (false, symptoms);
                    }
                };
            if let Err(error) = runtime_compile
                .stdin
                .take()
                .expect("runtime compiler stdin was piped")
                .write_all(GIN_RUNTIME_C.as_bytes())
            {
                let _ = runtime_compile.wait();
                let _ = std::fs::remove_file(&runtime_object);
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        format!("Failed to provide Gin runtime source: {error}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return (false, symptoms);
            }
            let runtime_result = match runtime_compile.wait_with_output() {
                Ok(result) => result,
                Err(error) => {
                    let _ = std::fs::remove_file(&runtime_object);
                    symptoms.push(
                        Diagnostic::new(
                            "codegen-internal",
                            format!("Failed to compile the Gin runtime: {error}"),
                        )
                        .with_help("an internal compiler error occurred")
                        .at_span(diagnostic::Span::new(0, 0)),
                    );
                    return (false, symptoms);
                }
            };
            if !runtime_result.status.success() {
                let stderr = String::from_utf8_lossy(&runtime_result.stderr);
                let _ = std::fs::remove_file(&runtime_object);
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        format!("Compiling the Gin runtime failed:\n{stderr}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return (false, symptoms);
            }
        }

        let mut cmd = Command::new(&cc);
        cmd.arg("-o").arg(output).arg(obj_path);
        if needs_runtime {
            cmd.arg(&runtime_object);
        }
        if let Some(triple) = target {
            cmd.arg(format!("--target={triple}"));
        }

        let result = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&runtime_object);
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
            let _ = std::fs::remove_file(&runtime_object);
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

        let _ = std::fs::remove_file(&runtime_object);
        (true, symptoms)
    }

    fn needs_legacy_runtime(obj_path: &Path, symptoms: &mut Vec<Diagnostic>) -> Option<bool> {
        let nm = Self::find_tool("llvm-nm", symptoms)?;
        let output = match Command::new(&nm)
            .args(["--undefined-only", "--just-symbol-name"])
            .arg(obj_path)
            .output()
        {
            Ok(output) if output.status.success() => output,
            result => {
                let reason = match result {
                    Ok(output) => String::from_utf8_lossy(&output.stderr).into_owned(),
                    Err(error) => error.to_string(),
                };
                symptoms.push(
                    Diagnostic::new(
                        "codegen-object-symbols",
                        format!("Failed to inspect object symbols with '{nm}': {reason}"),
                    )
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return None;
            }
        };
        Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(|symbol| {
                    matches!(
                        symbol.strip_prefix('_').unwrap_or(symbol),
                        "runtime_read_int" | "runtime_print_int" | "runtime_echo"
                    )
                }),
        )
    }

    pub fn object_cache_identity() -> Option<Vec<u8>> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        if std::env::var_os("CC").is_some() {
            return None;
        }
        let mut diagnostics = Vec::new();
        let tools = [
            Self::find_cc(&mut diagnostics)?,
            Self::find_tool("mlir-translate", &mut diagnostics)?,
        ];
        let mut hash = Sha256::new();
        for tool in tools {
            let path = if Path::new(&tool).is_absolute() {
                std::path::PathBuf::from(&tool)
            } else {
                std::env::split_paths(&std::env::var_os("PATH")?)
                    .map(|p| p.join(&tool))
                    .find(|p| p.is_file())?
            };
            let path = std::fs::canonicalize(path).ok()?;
            hash.update(path.as_os_str().as_encoded_bytes());
            let mut file = std::fs::File::open(&path).ok()?;
            let mut buffer = [0u8; 65536];
            loop {
                let size = file.read(&mut buffer).ok()?;
                if size == 0 {
                    break;
                }
                hash.update(&buffer[..size]);
            }
            let version = Command::new(&path).arg("--version").output().ok()?;
            if !version.status.success() {
                return None;
            }
            hash.update(version.stdout);
            hash.update(version.stderr);
        }
        let environment: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
        for (key, value) in environment {
            for bytes in [key.as_encoded_bytes(), value.as_encoded_bytes()] {
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
        }
        Some(hash.finalize().to_vec())
    }

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
            Diagnostic::new(
                "codegen-internal",
                format!("'{name}' not found. Install LLVM or add it to PATH."),
            )
            .with_help("an internal compiler error occurred")
            .at_span(diagnostic::Span::new(0, 0)),
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
                    Diagnostic::new(
                        "codegen-internal",
                        "No C compiler found. Install LLVM or set $CC.",
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                None
            }
        }
    }

    /// Translate LLVM-dialect MLIR text to LLVM IR using `mlir-translate`.
    pub(crate) fn mlir_to_llvm_ir(
        mlir_text: &str,
        symptoms: &mut Vec<Diagnostic>,
    ) -> Option<String> {
        let mlir_translate = Self::find_tool("mlir-translate", symptoms)?;

        let mut cmd = Command::new(&mlir_translate);
        cmd.arg("--mlir-to-llvmir");
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        format!("Failed to run mlir-translate: {e}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
                );
                return None;
            }
        };

        use std::io::Write;
        if let Err(e) = child.stdin.take().unwrap().write_all(mlir_text.as_bytes()) {
            symptoms.push(
                Diagnostic::new(
                    "codegen-internal",
                    format!("Failed to write to mlir-translate stdin: {e}"),
                )
                .with_help("an internal compiler error occurred")
                .at_span(diagnostic::Span::new(0, 0)),
            );
            return None;
        }

        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => {
                symptoms.push(
                    Diagnostic::new("codegen-internal", format!("mlir-translate failed: {e}"))
                        .with_help("an internal compiler error occurred")
                        .at_span(diagnostic::Span::new(0, 0)),
                );
                return None;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            symptoms.push(
                Diagnostic::new(
                    "codegen-internal",
                    format!(
                        "mlir-translate failed (exit {}):\n{stderr}",
                        output.status.code().unwrap_or(-1)
                    ),
                )
                .with_help("an internal compiler error occurred")
                .at_span(diagnostic::Span::new(0, 0)),
            );
            return None;
        }

        match String::from_utf8(output.stdout) {
            Ok(s) => Some(s),
            Err(e) => {
                symptoms.push(
                    Diagnostic::new(
                        "codegen-internal",
                        format!("mlir-translate output is not UTF-8: {e}"),
                    )
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
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
        fast: bool,
        target: Option<&str>,
        symptoms: &mut Vec<Diagnostic>,
    ) -> bool {
        let ll_path = obj_path.with_extension("ll");

        if let Err(e) = std::fs::write(&ll_path, llvm_ir) {
            symptoms.push(
                Diagnostic::new("codegen-internal", format!("Failed to write LLVM IR: {e}"))
                    .with_help("an internal compiler error occurred")
                    .at_span(diagnostic::Span::new(0, 0)),
            );
            return false;
        }

        let Some(cc) = Self::find_cc(symptoms) else {
            let _ = std::fs::remove_file(&ll_path);
            return false;
        };

        let opt_flag = if fast {
            "-O0"
        } else {
            match profile {
                Profile::Release => "-O2",
                Profile::Debug => "-O0",
            }
        };

        let mut command = Command::new(&cc);
        command
            .arg("-c")
            .arg(opt_flag)
            .arg(&ll_path)
            .arg("-o")
            .arg(obj_path);
        if let Some(triple) = target {
            command.arg(format!("--target={triple}"));
        }
        let result = match command.output() {
            Ok(o) => o,
            Err(e) => {
                symptoms.push(
                    Diagnostic::new("codegen-internal", format!("Failed to run '{cc}': {e}"))
                        .with_help("an internal compiler error occurred")
                        .at_span(diagnostic::Span::new(0, 0)),
                );
                let _ = std::fs::remove_file(&ll_path);
                return false;
            }
        };

        let _ = std::fs::remove_file(&ll_path);

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            symptoms.push(
                Diagnostic::new(
                    "codegen-internal",
                    format!(
                        "Compiling LLVM IR failed (exit {}):\n{stderr}",
                        result.status.code().unwrap_or(-1)
                    ),
                )
                .with_help("an internal compiler error occurred")
                .at_span(diagnostic::Span::new(0, 0)),
            );
            return false;
        }

        true
    }
}
