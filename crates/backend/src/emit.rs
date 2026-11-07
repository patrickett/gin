use diagnostic::Diagnostic;
use diagnostic::code::CodegenCode;
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
use std::path::Path;
use std::process::Command;
use strum::Display;

// AArch64 GlobalISel and its fallback disagree on stack slots for byte-array arguments.
const LLVM_CODEGEN_ARGUMENTS: [&str; 2] = ["-mllvm", "-global-isel=false"];

/// Build profile for optimization levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
pub enum Profile {
    #[default]
    #[strum(to_string = "debug")]
    Debug,
    #[strum(to_string = "release")]
    Release,
}

fn codegen_internal(message: impl Into<String>) -> Diagnostic {
    // codegen-internal
    Diagnostic::new(CodegenCode::Internal, message.into())
        .with_help("an internal compiler error occurred")
        .at_span(diagnostic::Span::new(0, 0))
}

fn run_tool(
    command: &mut Command,
    input: Option<&[u8]>,
    spawn_label: &str,
    status_message: impl FnOnce(&std::process::Output) -> String,
    symptoms: &mut Vec<Diagnostic>,
) -> Option<std::process::Output> {
    let result = match input {
        Some(input) => crate::process::run_with_input(command, input),
        None => command.output(),
    };
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            symptoms.push(codegen_internal(format!("{spawn_label}: {error}")));
            return None;
        }
    };
    if output.status.success() {
        return Some(output);
    }
    symptoms.push(codegen_internal(status_message(&output)));
    None
}

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
    /// Compile MLIR text (already verified by the caller) to a native object file.
    ///
    /// Each call is independent, so modules lowered separately can be emitted separately
    /// and joined by [`Self::link_executables`].
    pub fn native_from_mlir_text(
        &self,
        text: &str,
        obj_path: &Path,
        profile: Profile,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>) {
        self.native_from_mlir_text_with(text, obj_path, profile, false, target)
    }

    /// As [`Self::native_from_mlir_text`]; `fast` skips the MLIR optimization pipeline and
    /// compiles the LLVM IR with `-O0`, whatever the profile.
    pub fn native_from_mlir_text_with(
        &self,
        text: &str,
        obj_path: &Path,
        profile: Profile,
        fast: bool,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>) {
        let Some(module) = Module::parse(&self.context, text) else {
            return (
                false,
                vec![codegen_internal(
                    "Failed to parse MLIR text for native emission",
                )],
            );
        };
        let (ok, symptoms, _) = self
            .native_from_module_with_timings_for_target(&module, obj_path, profile, fast, target);
        (ok, symptoms)
    }

    /// Compile a pre-built MLIR `Module` to a native object file and return phase timings.
    pub fn native_from_module_with_timings_for_target(
        &self,
        module: &Module,
        obj_path: &Path,
        profile: Profile,
        fast: bool,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>, NativeCodegenTimings) {
        let _profile = compiler_profile::span(compiler_profile::Phase::NativeEmission);
        let mut symptoms = Vec::new();
        let mut timings = NativeCodegenTimings::default();
        let mut fixed_module = match self.reparse_for_backend(module) {
            Ok(module) => module,
            Err(diagnostic) => {
                symptoms.push(*diagnostic);
                return (false, symptoms, timings);
            }
        };

        let optimization_profile =
            compiler_profile::span(compiler_profile::Phase::MlirOptimization);
        let start = std::time::Instant::now();
        if !self.optimize(&mut fixed_module, profile, fast, &mut symptoms) {
            timings.optimization = start.elapsed();
            return (false, symptoms, timings);
        }
        timings.optimization = start.elapsed();
        drop(optimization_profile);

        let lowering_profile = compiler_profile::span(compiler_profile::Phase::LlvmLowering);
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
        drop(lowering_profile);

        let _emission_profile = compiler_profile::span(compiler_profile::Phase::ObjectEmission);
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
                symptoms.push(codegen_internal(format!("Optimization pass failed: {e}")));
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
                symptoms.push(codegen_internal(format!("MLIR pass pipeline failed: {e}")));
                false
            }
        }
    }

    pub(crate) fn reparse_for_backend<'a>(
        &'a self,
        module: &Module,
    ) -> Result<Module<'a>, Box<Diagnostic>> {
        crate::stack::verify_stack_storage(module).map_err(Box::new)?;
        let mlir_text = module.as_operation().to_string();
        Module::parse(&self.context, &mlir_text).ok_or_else(|| {
            Box::new(codegen_internal(
                "Failed to re-parse MLIR module for backend",
            ))
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

    /// Link an object file into an executable.
    pub fn link_executable(
        obj_path: &Path,
        output: &Path,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>) {
        Self::link_executables(&[obj_path], output, target)
    }

    /// Link several object files, one per source module, into an executable.
    pub fn link_executables(
        objects: &[&Path],
        output: &Path,
        target: Option<&str>,
    ) -> (bool, Vec<Diagnostic>) {
        assert!(!objects.is_empty(), "linking needs at least one object");
        let mut symptoms = Vec::new();

        let Some(cc) = Self::find_cc(&mut symptoms) else {
            return (false, symptoms);
        };
        // Runtime definitions must come from the supplied objects, not compiler helpers.
        let mut cmd = Command::new(&cc);
        cmd.arg("-o").arg(output).args(objects);
        if let Some(triple) = target {
            cmd.arg(format!("--target={triple}"));
        }

        let linked = run_tool(
            &mut cmd,
            None,
            &format!("Failed to run linker '{cc}'"),
            |result| {
                format!(
                    "Linking failed (exit {}):\n{}",
                    result.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&result.stderr)
                )
            },
            &mut symptoms,
        );
        if linked.is_none() {
            return (false, symptoms);
        }

        (true, symptoms)
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

        symptoms.push(codegen_internal(format!(
            "'{name}' not found. Install LLVM or add it to PATH."
        )));
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
                symptoms.push(codegen_internal(
                    "No C compiler found. Install LLVM or set $CC.",
                ));
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
        let output = run_tool(
            &mut cmd,
            Some(mlir_text.as_bytes()),
            "mlir-translate failed",
            |output| {
                format!(
                    "mlir-translate failed (exit {}):\n{}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stderr)
                )
            },
            symptoms,
        )?;

        match String::from_utf8(output.stdout) {
            Ok(s) => Some(s),
            Err(e) => {
                symptoms.push(codegen_internal(format!(
                    "mlir-translate output is not UTF-8: {e}"
                )));
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
        let Some(cc) = Self::find_cc(symptoms) else {
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
            .args(LLVM_CODEGEN_ARGUMENTS)
            .args(["-x", "ir", "-"])
            .arg("-o")
            .arg(obj_path);
        if let Some(triple) = target {
            command.arg(format!("--target={triple}"));
        }
        let result = run_tool(
            &mut command,
            Some(llvm_ir.as_bytes()),
            &format!("Failed to compile LLVM IR with '{cc}'"),
            |result| {
                format!(
                    "Compiling LLVM IR failed (exit {}):\n{}",
                    result.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&result.stderr)
                )
            },
            symptoms,
        );
        if result.is_none() {
            return false;
        }

        true
    }
}
