//! Native compilation: MLIR module → object file → linked executable.

// TODO: Add proper tail-call optimization. (unbounded tail calls in constant space).
// guarantee the same for self-recursive tail calls. recognize tail position, then use a
// jump instead of a call.
// maybe also greedy-shuffling to minimize argument-copying overhead in tail calls.

use diagnostic::Diagnostic;
use diagnostic::code::CodegenCode;
use melior::{Context, dialect::DialectRegistry, ir::Module, pass, utility};
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

    /// The input is reparsed into this compiler's context before optimization and lowering.
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
            let runtime_result = run_tool(
                &mut runtime_compile,
                Some(GIN_RUNTIME_C.as_bytes()),
                &format!("Failed to compile the Gin runtime with '{cc}'"),
                |result| {
                    format!(
                        "Compiling the Gin runtime failed:\n{}",
                        String::from_utf8_lossy(&result.stderr)
                    )
                },
                &mut symptoms,
            );
            if runtime_result.is_none() {
                let _ = std::fs::remove_file(&runtime_object);
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
            let _ = std::fs::remove_file(&runtime_object);
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
                    // codegen-object-symbols
                    Diagnostic::new(
                        CodegenCode::ObjectSymbols,
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

#[cfg(test)]
mod process_tests {
    use super::*;
    use melior::ir::operation::OperationLike;

    #[test]
    fn backend_reparse_rejects_invalid_llvm_call_segments() {
        use melior::ir::attribute::{DenseI32ArrayAttribute, FlatSymbolRefAttribute};
        use melior::ir::operation::OperationBuilder;
        use melior::ir::r#type::IntegerType;
        use melior::ir::{BlockLike, Identifier, Location, RegionLike};
        let context = NativeCompiler::create_context();
        let module = Module::parse(
            &context,
            r#"module {
            llvm.func @callee(i64) -> i64
            llvm.func @caller(%value: i64) -> i64 {
                llvm.return %value : i64
            }
        }"#,
        )
        .unwrap();
        let caller = module
            .body()
            .first_operation()
            .unwrap()
            .next_in_block()
            .unwrap();
        let block = caller.region(0).unwrap().first_block().unwrap();
        let call = OperationBuilder::new("llvm.call", Location::unknown(&context))
            .add_attributes(&[
                (
                    Identifier::new(&context, "callee"),
                    FlatSymbolRefAttribute::new(&context, "callee").into(),
                ),
                (
                    Identifier::new(&context, "op_bundle_sizes"),
                    DenseI32ArrayAttribute::new(&context, &[]).into(),
                ),
            ])
            .add_operands(&[block.argument(0).unwrap().into()])
            .add_results(&[IntegerType::new(&context, 64).into()])
            .build()
            .unwrap();
        block.insert_operation(0, call);
        assert!(!module.as_operation().verify());
        assert!(
            NativeCompiler::default()
                .reparse_for_backend(&module)
                .is_err()
        );
    }

    #[test]
    fn backend_reparse_preserves_indirect_llvm_calls() {
        let context = NativeCompiler::create_context();
        let module = Module::parse(
            &context,
            r#"module {
            llvm.func @caller(%callee: !llvm.ptr, %value: i64) -> i64 {
                %result = llvm.call %callee(%value) : !llvm.ptr, (i64) -> i64
                llvm.return %result : i64
            }
        }"#,
        )
        .unwrap();
        assert!(module.as_operation().verify());
        let (llvm, diagnostics) =
            NativeCompiler::default().llvm_ir_from_module(&module, Profile::Debug, true);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let llvm = llvm.unwrap();
        assert!(llvm.contains("call i64 %"), "{llvm}");
    }

    #[test]
    fn backend_reparse_preserves_llvm_call_operand_bundles() {
        let context = NativeCompiler::create_context();
        let module = Module::parse(
            &context,
            r#"module {
            llvm.func @callee(i64) -> i64
            llvm.func @caller(%value: i64) -> i64 {
                %result = "llvm.call"(%value, %value) <{
                    callee = @callee,
                    operandSegmentSizes = array<i32: 1, 1>,
                    op_bundle_sizes = array<i32: 1>,
                    op_bundle_tags = ["deopt"]
                }> : (i64, i64) -> i64
                llvm.return %result : i64
            }
        }"#,
        )
        .unwrap();
        assert!(module.as_operation().verify());
        let compiler = NativeCompiler::default();
        let (llvm, diagnostics) = compiler.llvm_ir_from_module(&module, Profile::Debug, true);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let llvm = llvm.unwrap();
        assert!(llvm.contains("call i64 @callee(i64"), "{llvm}");
        assert!(llvm.contains("\"deopt\"(i64"), "{llvm}");
    }

    #[test]
    fn translates_llvm_module_through_piped_input() {
        let mut diagnostics = Vec::new();
        let output = NativeCompiler::mlir_to_llvm_ir(
            r#"module {
                llvm.func @answer() -> i64 {
                    %result = llvm.mlir.constant(42 : i64) : i64
                    llvm.return %result : i64
                }
            }"#,
            &mut diagnostics,
        )
        .unwrap_or_else(|| panic!("translation failed: {diagnostics:?}"));
        assert!(diagnostics.is_empty());
        assert!(output.contains("define i64 @answer()"));
        assert!(output.contains("ret i64 42"));
    }

    #[test]
    fn failed_translation_captures_tool_diagnostic() {
        let mut diagnostics = Vec::new();
        assert!(NativeCompiler::mlir_to_llvm_ir("invalid module", &mut diagnostics).is_none());
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("mlir-translate failed"));
        assert!(diagnostics[0].message.contains("error:"));
    }

    #[test]
    fn object_compilation_preserves_neighboring_llvm_ir() {
        for (profile, fast, valid) in [
            (Profile::Debug, false, true),
            (Profile::Release, false, true),
            (Profile::Release, true, true),
            (Profile::Debug, false, false),
        ] {
            let package = test_fixtures::TempPackage::new("codegen_neighboring_ir");
            let object = package.join("answer.o");
            let neighboring_ir = package.write("answer.ll", "user-owned LLVM IR");
            let mut diagnostics = Vec::new();
            let input = if valid {
                "define i32 @main() { ret i32 42 }"
            } else {
                "invalid LLVM IR"
            };
            let compiled = NativeCompiler::compile_llvm_ir_to_object(
                input,
                &object,
                profile,
                fast,
                None,
                &mut diagnostics,
            );
            assert_eq!(compiled, valid, "{diagnostics:?}");
            assert_eq!(
                std::fs::read_to_string(neighboring_ir).unwrap(),
                "user-owned LLVM IR"
            );
            if valid {
                assert!(diagnostics.is_empty());
                assert!(object.is_file());
                #[cfg(unix)]
                {
                    let executable = package.join("answer");
                    let (linked, diagnostics) =
                        NativeCompiler::link_executable(&object, &executable, None);
                    assert!(linked, "{diagnostics:?}");
                    let output = Command::new(executable).output().unwrap();
                    assert_eq!(output.status.code(), Some(42));
                    assert!(output.stdout.is_empty());
                    assert!(output.stderr.is_empty());
                }
            } else {
                assert_eq!(diagnostics.len(), 1);
                assert!(diagnostics[0].message.contains("error:"));
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn compiles_and_links_legacy_runtime_from_stdin() {
        let package = test_fixtures::TempPackage::new("codegen_legacy_runtime_stdin");
        let object = package.join("answer.o");
        let executable = package.join("answer");
        let mut diagnostics = Vec::new();
        assert!(
            NativeCompiler::compile_llvm_ir_to_object(
                "declare i64 @runtime_print_int(i64)\ndefine i32 @main() {\n%result = call i64 @runtime_print_int(i64 42)\nret i32 0\n}\n",
                &object,
                Profile::Debug,
                true,
                None,
                &mut diagnostics,
            ),
            "{diagnostics:?}"
        );
        assert!(diagnostics.is_empty());
        let (linked, diagnostics) = NativeCompiler::link_executable(&object, &executable, None);
        assert!(linked, "{diagnostics:?}");
        assert!(diagnostics.is_empty());
        let output = Command::new(executable).output().unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"42\n");
        assert!(output.stderr.is_empty());
        assert!(!std::fs::read_dir(package.join("")).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("gin-runtime-")
        }));
    }
}
