use crate::runtime::{
    GenerationId, LiveRuntime, NativeExport, NativeGeneration, RuntimeDeclarationKind,
    RuntimeFunction, RuntimeStorage, RuntimeType, RuntimeValue, generation_wrapper_name,
    runtime_symbol_name,
};
use crate::source::RuntimeDeclaration;
use crate::{GeneratedEvaluation, InterpreterContext, InterpreterDiagnostic, SessionSource};
use ast::ty::IntegerInterpretation;
use ast_format::declare::BindFormatExt;
use codegen::{JitError, NativeCompiler, NativeEngine};
use diagnostic::{Category, Diagnostic, DiagnosticPathExt, Span};
use ginc::driver::{
    CheckedFile, CheckedPackage, CompilePackage, CompilerDriver, LoweredFileDiagnostics,
    SourceFile, SourcePackage,
};
use parser::query::SourceParseExt;
use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Clear,
    Reset,
    Help,
    Quit,
    Tasks,
    TaskStarted(TaskId),
    TaskStopRequested(TaskId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::From)]
pub struct TaskId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Cancelled,
    Completed,
    Failed,
}

#[derive(Debug)]
pub struct TaskSummary {
    pub id: TaskId,
    pub state: TaskState,
    pub expression: String,
}

#[derive(Debug)]
pub struct TaskResult {
    pub id: TaskId,
    pub expression: String,
    pub state: TaskState,
    pub submission: Submission,
}

struct SpawnedTask {
    expression: String,
    stop_requested: Arc<AtomicBool>,
    active_generations: crate::runtime::ActiveGenerations,
    receiver: Receiver<TaskResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionKind {
    Declaration,
    Expression,
    Mixed,
}

#[derive(Debug)]
pub enum Submission {
    DeclarationAccepted {
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    Evaluated {
        value: RuntimeValue,
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    Command(Command),
    Empty,
    Incomplete,
    Rejected {
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    UnknownCommand(String),
}

pub struct InterpreterSession {
    context: InterpreterContext,
    source: SessionSource,
    next_generation: u64,
    next_runtime_generation: u64,
    next_task_id: u64,
    live_runtime: LiveRuntime,
    running_tasks: HashMap<TaskId, SpawnedTask>,
    driver: CompilerDriver,
}

const RUNTIME_I64_MIN: &str = "-9223372036854775808";
const RUNTIME_I64_MAX: &str = "9223372036854775807";
const GIN_CORE_PRELUDE: &str =
    "use 'primitive/'.(SignedBigInt, signed_big_int_add_bits, signed_big_int_add)";

impl Default for InterpreterSession {
    fn default() -> Self {
        Self::with_context(InterpreterContext::isolated())
    }
}

impl InterpreterSession {
    pub fn standalone() -> Result<Self, crate::InterpreterContextError> {
        let mut session = Self::with_context(InterpreterContext::gin_core()?);
        session.source = SessionSource::default().with_evaluation_type("SignedBigInt");
        let prelude = session.submit(GIN_CORE_PRELUDE);
        assert!(
            matches!(prelude, Submission::DeclarationAccepted { .. }),
            "gin_core interpreter prelude should compile: {prelude:#?}"
        );
        Ok(session)
    }

    pub fn with_context(context: InterpreterContext) -> Self {
        Self {
            context,
            source: SessionSource::default(),
            next_generation: 1,
            next_runtime_generation: 1,
            next_task_id: 1,
            running_tasks: HashMap::new(),
            live_runtime: LiveRuntime::default(),
            driver: CompilerDriver::default(),
        }
    }

    pub fn context(&self) -> &InterpreterContext {
        &self.context
    }

    pub fn declarations(&self) -> &str {
        self.source.declarations()
    }

    #[allow(dead_code)]
    pub(crate) fn live_runtime(&self) -> &LiveRuntime {
        &self.live_runtime
    }

    #[allow(dead_code)]
    pub(crate) fn live_runtime_mut(&mut self) -> &mut LiveRuntime {
        &mut self.live_runtime
    }

    pub fn task_summaries(&self) -> Vec<TaskSummary> {
        self.running_tasks
            .iter()
            .map(|task| TaskSummary {
                id: *task.0,
                state: TaskState::Running,
                expression: task.1.expression.clone(),
            })
            .collect()
    }

    pub fn poll_task_results(&mut self) -> Vec<TaskResult> {
        let mut completed = Vec::new();
        let task_ids: Vec<_> = self.running_tasks.keys().copied().collect();
        let mut done = Vec::new();

        for task_id in task_ids {
            let task = match self.running_tasks.get_mut(&task_id) {
                Some(task) => task,
                None => continue,
            };

            match task.receiver.try_recv() {
                Ok(result) => {
                    self.live_runtime
                        .finish_active_generations(task.active_generations.clone());
                    completed.push(result);
                    done.push(task_id);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.live_runtime
                        .finish_active_generations(task.active_generations.clone());
                    completed.push(TaskResult {
                        id: task_id,
                        expression: task.expression.clone(),
                        state: TaskState::Failed,
                        submission: Submission::Rejected {
                            diagnostics: vec![submitted_diagnostic(
                                "",
                                Diagnostic::new(
                                    "interpreter-task-failure",
                                    "task worker terminated unexpectedly",
                                ),
                            )],
                        },
                    });
                    done.push(task_id);
                }
            }
        }

        for task_id in done {
            self.running_tasks.remove(&task_id);
        }

        completed
    }

    pub fn stop_task(&mut self, task_id: TaskId) -> bool {
        let Some(task) = self.running_tasks.get_mut(&task_id) else {
            return false;
        };

        task.stop_requested.store(true, Ordering::SeqCst);
        true
    }

    pub fn clear_tasks(&mut self) {
        for task in self.running_tasks.values() {
            task.stop_requested.store(true, Ordering::SeqCst);
        }
        self.running_tasks.clear();
    }

    fn spawn_task(&mut self, expression: &str) -> TaskId {
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("interpreter evaluation generation overflow");
        let evaluation = self.source.evaluation_candidate_with_dispatches(
            expression,
            generation,
            self.live_runtime.symbols(),
        );
        let live_symbols = self.live_runtime.symbols().clone();
        let active_generations = self
            .live_runtime
            .track_active_generations_for_symbols(&live_symbols);
        let context = self.context.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_token = stop.clone();
        let id = TaskId(self.next_task_id);
        self.next_task_id = self
            .next_task_id
            .checked_add(1)
            .expect("interpreter task id overflow");

        let (sender, receiver) = mpsc::channel();

        let task_id = id;
        thread::spawn(move || {
            let result = run_task_submission(context, evaluation, task_id, stop_token);
            let _ = sender.send(result);
        });

        self.running_tasks.insert(
            id,
            SpawnedTask {
                expression: expression.to_string(),
                stop_requested: stop,
                active_generations,
                receiver,
            },
        );

        id
    }

    pub fn submit(&mut self, input: &str) -> Submission {
        let input = input.trim_end();
        if input.trim().is_empty() {
            return Submission::Empty;
        }

        if let Some(command) = input.trim().strip_prefix('#') {
            let name = command.split_whitespace().next();
            if matches!(
                name,
                Some("clear" | "reset" | "tasks" | "stop" | "run" | "help" | "exit" | "quit" | "q")
            ) {
                return self.command(command);
            }
        }

        let parsed = input.parse_source_full();
        if parsed.is_incomplete() {
            return Submission::Incomplete;
        }

        let declaration_names = declaration_names(&parsed.ast);
        let user_source = match SubmissionKind::from_ast(&parsed.ast) {
            SubmissionKind::Declaration => self
                .source
                .candidate_without_shadowed(input, &declaration_names),
            SubmissionKind::Expression => self.source.candidate(input),
            SubmissionKind::Mixed => self.source.candidate(input),
        };
        if compiler_has_flaws(&parsed.symptoms) {
            return Submission::Rejected {
                diagnostics: submitted_diagnostics(input, parsed.symptoms),
            };
        }

        match SubmissionKind::from_ast(&parsed.ast) {
            SubmissionKind::Mixed => Submission::Rejected {
                diagnostics: submitted_diagnostics(
                    input,
                    vec![Diagnostic::new(
                        "interpreter-mixed-submission",
                        "declarations and expressions must be submitted separately",
                    )],
                ),
            },
            SubmissionKind::Declaration => {
                self.submit_declaration(input, user_source, declaration_names, &parsed.ast)
            }
            SubmissionKind::Expression => self.evaluate(input),
        }
    }

    fn submit_declaration(
        &mut self,
        input: &str,
        candidate: String,
        declaration_names: Vec<String>,
        ast: &ast::FileAst,
    ) -> Submission {
        let checked = self.check(&candidate);
        let mut diagnostics = package_diagnostics(&checked, None);
        if checked.has_front_end_flaws() || checked.has_typecheck_flaws() {
            return Submission::Rejected { diagnostics };
        }

        let runtime_metadata = declaration_runtime_metadata(&checked, &declaration_names);
        let mut compatible = true;
        for declaration in &runtime_metadata {
            let Some(existing_value) = self.live_runtime.value_for_symbol(&declaration.name) else {
                continue;
            };
            if !declarations_compatible(existing_value, declaration) {
                diagnostics.push(submitted_diagnostic(
                    &candidate,
                    incompatible_redefinition_diagnostic(
                        &declaration.name,
                        existing_value,
                        declaration,
                    ),
                ));
                compatible = false;
            }
        }
        if !compatible {
            return Submission::Rejected { diagnostics };
        }

        if declaration_names.is_empty() {
            self.source.accept_declaration(input, declaration_names);
            return Submission::DeclarationAccepted { diagnostics };
        }

        let context = NativeCompiler::create_context();
        let lowered = self.driver.lower_package(&context, &checked);
        diagnostics.extend(lowered_diagnostics(&checked, lowered.diagnostics, None));
        if has_flaws(&diagnostics) || lowered.module.is_none() {
            if lowered.module.is_none() && !has_flaws(&diagnostics) {
                diagnostics.push(submitted_diagnostic(
                    &candidate,
                    codegen_failure("code generation produced no module"),
                ));
            }
            return Submission::Rejected { diagnostics };
        }

        let generation_id = GenerationId(self.next_runtime_generation);
        let generation_symbol = generation_wrapper_name(generation_id);
        let generation_candidate =
            generation_candidate(&candidate, &generation_symbol, generation_id);
        let runtime_engine =
            self.build_runtime_generation(&generation_candidate, &generation_symbol);

        let runtime_declarations =
            extract_runtime_declarations(ast, checked.primary_file().typed());
        let declared_logical_symbols: Vec<_> = runtime_declarations
            .iter()
            .map(|declaration| declaration.name.clone())
            .collect();
        let exports = generation_exports(&declared_logical_symbols, generation_id);
        self.live_runtime.install_generation(NativeGeneration {
            id: generation_id,
            engine: runtime_engine,
            exports,
        });
        self.next_runtime_generation = self
            .next_runtime_generation
            .checked_add(1)
            .expect("interpreter runtime generation overflow");

        self.source.accept_declaration_with_dispatches(
            input,
            declaration_names,
            runtime_declarations,
        );
        self.live_runtime.retire_unused_generations();

        for declaration in runtime_metadata {
            let runtime_value = match declaration.kind {
                RuntimeDeclarationKind::Value => RuntimeValue::new(
                    declaration.return_type,
                    RuntimeStorage::zero(usize::from(declaration.return_type.size)),
                ),
                RuntimeDeclarationKind::Function => RuntimeValue::new_function(
                    declaration.return_type,
                    declaration.parameter_types,
                    declaration.param_conventions,
                    declaration.capture_layout,
                ),
            };
            self.live_runtime
                .store_value(declaration.name, runtime_value);
        }

        Submission::DeclarationAccepted { diagnostics }
    }

    fn evaluate(&mut self, input: &str) -> Submission {
        let expression_checked = self.check(&self.source.candidate(input));
        let result_type_diagnostic = if expression_checked.has_front_end_flaws()
            || expression_checked.has_typecheck_flaws()
        {
            None
        } else {
            invalid_evaluation_result_type(expression_checked.primary_file(), input)
        };

        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("interpreter evaluation generation overflow");
        let evaluation = self.source.evaluation_candidate_with_dispatches(
            input,
            generation,
            self.live_runtime.symbols(),
        );
        let symbol = evaluation.symbol().to_string();
        let checked = self.check(evaluation.source());
        let mut diagnostics = package_diagnostics(&checked, Some(&evaluation));
        if let Some(diagnostic) = result_type_diagnostic {
            diagnostics.push(submitted_diagnostic(input, diagnostic));
            return Submission::Rejected { diagnostics };
        }
        if checked.has_front_end_flaws() || checked.has_typecheck_flaws() {
            return Submission::Rejected { diagnostics };
        }

        let context = NativeCompiler::create_context();
        let lowered = self.driver.lower_package(&context, &checked);
        diagnostics.extend(lowered_diagnostics(
            &checked,
            lowered.diagnostics,
            Some(&evaluation),
        ));
        if has_flaws(&diagnostics) {
            return Submission::Rejected { diagnostics };
        }
        let Some(module) = lowered.module else {
            diagnostics.push(generated_diagnostic(
                &evaluation,
                codegen_failure("code generation produced no module"),
            ));
            return Submission::Rejected { diagnostics };
        };

        let engine = match NativeEngine::new(module, &symbol) {
            Ok(engine) => engine,
            Err(error) => {
                diagnostics.push(jit_diagnostic(&evaluation, error));
                return Submission::Rejected { diagnostics };
            }
        };
        let live_symbols = self.live_runtime.symbols().clone();
        let active_generations = self
            .live_runtime
            .track_active_generations_for_symbols(&live_symbols);
        let value = match engine.invoke_i64() {
            Ok(value) => value,
            Err(error) => {
                self.live_runtime
                    .finish_active_generations(active_generations);
                diagnostics.push(jit_diagnostic(&evaluation, error));
                return Submission::Rejected { diagnostics };
            }
        };
        self.live_runtime
            .finish_active_generations(active_generations);

        Submission::Evaluated {
            value: RuntimeValue::new(RuntimeType::I64, RuntimeStorage::Int64(value)),
            diagnostics,
        }
    }

    fn check(&mut self, source: &str) -> CheckedPackage {
        let session_path = self.context.package().session_path().to_path_buf();
        let mut sources = self.context.package().sources().to_vec();
        sources.push(SourceFile::new(session_path.clone(), source.to_string()));
        let parsed = self
            .driver
            .parse(SourcePackage::new(session_path, sources))
            .expect("interpreter includes its primary session source");
        let package = CompilePackage::new(parsed, self.context.target().clone())
            .with_dependencies(self.context.package().dependencies().clone())
            .with_import_resolution(true);
        self.driver.check(package)
    }

    fn command(&mut self, command: &str) -> Submission {
        let command = command.trim();
        let mut parts = command.splitn(2, ' ');
        match parts.next() {
            Some("clear") => {
                self.source.clear();
                self.live_runtime.clear();
                Submission::Command(Command::Clear)
            }
            Some("reset") => {
                self.source.clear();
                self.live_runtime.clear();
                self.clear_tasks();
                Submission::Command(Command::Reset)
            }
            Some("tasks") => Submission::Command(Command::Tasks),
            Some("stop") => match parts.next().and_then(|id| id.parse::<u64>().ok()) {
                Some(task_id) if self.stop_task(TaskId(task_id)) => {
                    Submission::Command(Command::TaskStopRequested(TaskId(task_id)))
                }
                Some(_) => Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        command,
                        Diagnostic::new("interpreter-invalid-task-id", "unknown task id"),
                    )],
                },
                None => Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        command,
                        Diagnostic::new(
                            "interpreter-invalid-command-usage",
                            "usage: #stop <task-id>",
                        ),
                    )],
                },
            },
            Some("run") => match parts
                .next()
                .map(str::trim)
                .filter(|expression| !expression.is_empty())
            {
                Some(expression) => {
                    let task_id = self.spawn_task(expression);
                    Submission::Command(Command::TaskStarted(task_id))
                }
                None => Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        command,
                        Diagnostic::new(
                            "interpreter-invalid-command-usage",
                            "usage: #run <expression>",
                        ),
                    )],
                },
            },
            Some("help") => Submission::Command(Command::Help),
            Some("exit") | Some("quit") | Some("q") => Submission::Command(Command::Quit),
            Some(other) => Submission::UnknownCommand(other.to_string()),
            None => Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    command,
                    Diagnostic::new("interpreter-invalid-command", "unknown command"),
                )],
            },
        }
    }

    fn build_runtime_generation(
        &mut self,
        generation_candidate: &str,
        generation_symbol: &str,
    ) -> Option<NativeEngine> {
        let context = NativeCompiler::create_context();
        let runtime_checked = self.check(generation_candidate);
        if runtime_checked.has_front_end_flaws() || runtime_checked.has_typecheck_flaws() {
            return None;
        }

        let runtime_lowered = self.driver.lower_package(&context, &runtime_checked);
        let runtime_module = runtime_lowered.module?;

        NativeEngine::new(runtime_module, generation_symbol).ok()
    }
}

fn run_task_submission(
    context: InterpreterContext,
    evaluation: GeneratedEvaluation,
    task_id: TaskId,
    stop_requested: Arc<AtomicBool>,
) -> TaskResult {
    let expression = evaluation.expression().to_string();
    let expression_for_worker = expression.clone();
    let worker = std::panic::AssertUnwindSafe(move || {
        let mut driver = ginc::driver::CompilerDriver::default();
        let session_path = context.package().session_path().to_path_buf();
        let evaluation_source = evaluation.source().to_string();
        let symbol = evaluation.symbol().to_string();
        let expression = expression_for_worker.clone();

        if stop_requested.load(Ordering::SeqCst) {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Cancelled,
                submission: Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        "<interpreter>",
                        Diagnostic::new("interpreter-task-cancelled", "task was cancelled"),
                    )],
                },
            };
        }

        let mut sources = context.package().sources().to_vec();
        sources.push(SourceFile::new(session_path, evaluation_source));
        let parsed = match driver.parse(ginc::driver::SourcePackage::new(
            context.package().session_path().to_path_buf(),
            sources,
        )) {
            Ok(parsed) => parsed,
            Err(error) => {
                return TaskResult {
                    id: task_id,
                    expression: expression.clone(),
                    state: TaskState::Failed,
                    submission: Submission::Rejected {
                        diagnostics: vec![submitted_diagnostic(
                            "<interpreter>",
                            Diagnostic::new("interpreter-task-failed", error.to_string()),
                        )],
                    },
                };
            }
        };

        let package = CompilePackage::new(parsed, context.target().clone())
            .with_dependencies(context.package().dependencies().clone())
            .with_import_resolution(true);
        let checked = driver.check(package);
        let mut diagnostics = package_diagnostics(&checked, Some(&evaluation));
        if checked.has_front_end_flaws() || checked.has_typecheck_flaws() {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Failed,
                submission: Submission::Rejected { diagnostics },
            };
        }

        if stop_requested.load(Ordering::SeqCst) {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Cancelled,
                submission: Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        "<interpreter>",
                        Diagnostic::new("interpreter-task-cancelled", "task was cancelled"),
                    )],
                },
            };
        }

        let context = NativeCompiler::create_context();
        let lowered = driver.lower_package(&context, &checked);
        diagnostics.extend(lowered_diagnostics(
            &checked,
            lowered.diagnostics,
            Some(&evaluation),
        ));
        if has_flaws(&diagnostics) {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Failed,
                submission: Submission::Rejected { diagnostics },
            };
        }

        let Some(module) = lowered.module else {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Failed,
                submission: Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        "<interpreter>",
                        codegen_failure("code generation produced no module"),
                    )],
                },
            };
        };

        if stop_requested.load(Ordering::SeqCst) {
            return TaskResult {
                id: task_id,
                expression: expression.clone(),
                state: TaskState::Cancelled,
                submission: Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        "<interpreter>",
                        Diagnostic::new("interpreter-task-cancelled", "task was cancelled"),
                    )],
                },
            };
        }

        let engine = match NativeEngine::new(module, &symbol) {
            Ok(engine) => engine,
            Err(error) => {
                return TaskResult {
                    id: task_id,
                    expression: expression.clone(),
                    state: TaskState::Failed,
                    submission: Submission::Rejected {
                        diagnostics: vec![submitted_diagnostic(
                            &expression,
                            Diagnostic::new("interpreter-runtime-failure", error.to_string()),
                        )],
                    },
                };
            }
        };

        let value = match engine.invoke_i64() {
            Ok(value) => value,
            Err(error) => {
                return TaskResult {
                    id: task_id,
                    expression: expression.clone(),
                    state: TaskState::Failed,
                    submission: Submission::Rejected {
                        diagnostics: vec![jit_diagnostic(&evaluation, error)],
                    },
                };
            }
        };

        TaskResult {
            id: task_id,
            expression,
            state: TaskState::Completed,
            submission: Submission::Evaluated {
                value: RuntimeValue::new(RuntimeType::I64, RuntimeStorage::Int64(value)),
                diagnostics,
            },
        }
    });

    match std::panic::catch_unwind(worker) {
        Ok(result) => result,
        Err(_) => TaskResult {
            id: task_id,
            expression,
            state: TaskState::Failed,
            submission: Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    "<interpreter>",
                    Diagnostic::new("interpreter-task-failed", "task panicked"),
                )],
            },
        },
    }
}

impl SubmissionKind {
    fn from_ast(ast: &ast::FileAst) -> Self {
        let has_declarations = !ast.uses.is_empty()
            || !ast.tags.is_empty()
            || !ast.defs.is_empty()
            || !ast.method_binds.is_empty()
            || !ast.symbol_aliases.is_empty();
        let has_expressions = !ast.exprs.is_empty();

        match (has_declarations, has_expressions) {
            (true, true) => Self::Mixed,
            (false, true) => Self::Expression,
            _ => Self::Declaration,
        }
    }
}

fn declaration_names(ast: &ast::FileAst) -> Vec<String> {
    let mut names = Vec::new();

    names.extend(ast.tags.keys().map(|name| name.to_string()));
    names.extend(ast.defs.keys().map(|name| name.to_string()));
    names.extend(ast.method_binds.iter().map(|bind| bind.name.to_string()));
    names.extend(
        ast.symbol_aliases
            .iter()
            .map(|alias| alias.alias.to_string()),
    );

    names.sort();
    names.dedup();
    names
}

#[derive(Debug, Clone)]
struct DeclarationRuntimeMetadata {
    name: String,
    kind: RuntimeDeclarationKind,
    return_type: RuntimeType,
    parameter_types: Vec<RuntimeType>,
    param_conventions: Vec<ast::ParamConvention>,
    capture_layout: Vec<RuntimeType>,
}

fn declaration_runtime_metadata(
    checked: &CheckedPackage,
    declaration_names: &[String],
) -> Vec<DeclarationRuntimeMetadata> {
    let declaration_names: std::collections::HashSet<String> =
        declaration_names.iter().cloned().collect();
    let typed = checked.primary_file().typed();
    typed
        .defs
        .values()
        .filter_map(|declaration| {
            if !declaration_names.contains(declaration.name.as_ref()) {
                return None;
            }
            declaration_runtime_type(&declaration.return_type, &typed.type_registry).map(
                |return_type| DeclarationRuntimeMetadata {
                    name: declaration.name.as_ref().to_string(),
                    kind: if declaration.is_constant {
                        RuntimeDeclarationKind::Value
                    } else {
                        RuntimeDeclarationKind::Function
                    },
                    return_type,
                    parameter_types: declaration
                        .params
                        .iter()
                        .filter_map(|(_, ty)| declaration_runtime_type(ty, &typed.type_registry))
                        .collect(),
                    param_conventions: declaration.param_conventions.clone(),
                    capture_layout: Vec::new(),
                },
            )
        })
        .collect()
}

fn declarations_compatible(
    existing: &RuntimeValue,
    replacement: &DeclarationRuntimeMetadata,
) -> bool {
    match (&existing.kind, &replacement.kind) {
        (RuntimeDeclarationKind::Value, RuntimeDeclarationKind::Value) => {
            existing.ty.is_compatible_with(&replacement.return_type)
        }
        (RuntimeDeclarationKind::Function, RuntimeDeclarationKind::Function) => {
            let Some(existing_function) = existing.function.as_ref() else {
                return false;
            };
            let replacement_function = RuntimeFunction {
                return_type: replacement.return_type,
                parameter_types: replacement.parameter_types.clone(),
                parameter_conventions: replacement.param_conventions.clone(),
                capture_layout: replacement.capture_layout.clone(),
            };
            existing_function.is_compatible_with(&replacement_function)
        }
        _ => false,
    }
}

fn declaration_runtime_type(
    return_type: &ast::ty::Ty,
    registry: &typecheck::TypeRegistry,
) -> Option<RuntimeType> {
    let width = u8::try_from(registry.integer_width_for_type(return_type)?).ok()?;
    let size = width.div_ceil(8);
    if size == 0 {
        return None;
    }
    Some(RuntimeType {
        size,
        align: integer_alignment(size),
    })
}

fn integer_alignment(size: u8) -> u8 {
    let size = u64::from(size);
    let align = size.next_power_of_two().clamp(1, 16);
    u8::try_from(align).expect("runtime alignment is always small")
}

fn incompatible_redefinition_diagnostic(
    symbol: &str,
    existing: &RuntimeValue,
    replacement: &DeclarationRuntimeMetadata,
) -> Diagnostic {
    let replacement_description = match replacement.kind {
        RuntimeDeclarationKind::Value => "value".to_string(),
        RuntimeDeclarationKind::Function => format!(
            "function return {:?} with params {:?} and conventions {:?}",
            replacement.return_type, replacement.parameter_types, replacement.param_conventions
        ),
    };
    let existing_description = match existing.kind {
        RuntimeDeclarationKind::Value => "value".to_string(),
        RuntimeDeclarationKind::Function => {
            let function = existing.function.as_ref().map(|existing_function| {
                format!(
                    "function return {:?} with params {:?} and conventions {:?}",
                    existing_function.return_type,
                    existing_function.parameter_types,
                    existing_function.parameter_conventions
                )
            });
            function
                .as_deref()
                .unwrap_or("function (missing metadata)")
                .to_string()
        }
    };

    Diagnostic::new(
        "interpreter-incompatible-redefinition",
        format!(
            "cannot redefine `{symbol}` because declaration changed from `{existing_description}` to `{replacement_description}`"
        ),
    )
    .with_arg("symbol", symbol)
    .with_arg("existing_type", format!("{:?}", existing.ty))
    .with_arg("existing_kind", format!("{:?}", existing.kind))
    .with_arg(
        "replacement_type",
        match replacement.kind {
            RuntimeDeclarationKind::Value => format!("{:?}", replacement.return_type),
            RuntimeDeclarationKind::Function => format!(
                "fn({:?}) -> {:?}",
                replacement.param_conventions,
                replacement.return_type
            ),
        },
    )
    .with_arg("replacement_kind", format!("{:?}", replacement.kind))
}

fn extract_runtime_declarations(
    ast: &ast::FileAst,
    typed: &typecheck::TypedFileAst,
) -> Vec<RuntimeDeclaration> {
    let mut declarations = Vec::new();

    for bind in ast.defs.values() {
        let param_conventions = bind
            .params
            .as_ref()
            .map(|params| {
                params
                    .keys()
                    .map(|name| {
                        bind.param_conventions
                            .get(name)
                            .copied()
                            .unwrap_or(ast::ParamConvention::Own)
                    })
                    .collect()
            })
            .unwrap_or_default();

        declarations.push(RuntimeDeclaration::new(
            bind.name.as_str().to_string(),
            runtime_signature(bind, typed),
            bind.params
                .as_ref()
                .map(|params| params.keys().map(|name| name.to_string()).collect())
                .unwrap_or_default(),
            param_conventions,
            bind.is_constant(),
        ));
    }

    for bind in &ast.method_binds {
        let param_conventions = bind
            .params
            .as_ref()
            .map(|params| {
                params
                    .keys()
                    .map(|name| {
                        bind.param_conventions
                            .get(name)
                            .copied()
                            .unwrap_or(ast::ParamConvention::Own)
                    })
                    .collect()
            })
            .unwrap_or_default();

        declarations.push(RuntimeDeclaration::new(
            bind.name.as_str().to_string(),
            runtime_signature(bind, typed),
            bind.params
                .as_ref()
                .map(|params| params.keys().map(|name| name.to_string()).collect())
                .unwrap_or_default(),
            param_conventions,
            bind.is_constant(),
        ));
    }

    declarations
}

fn runtime_signature(bind: &ast::Bind, typed: &typecheck::TypedFileAst) -> String {
    let mut signature = bind.signature_surface();
    if bind.return_tag.is_none()
        && bind.return_type_name.is_none()
        && let Some(return_type) = typed
            .defs
            .values()
            .find(|typed_bind| typed_bind.name == bind.name)
            .map(|typed_bind| &typed_bind.return_type)
    {
        signature.push(' ');
        signature.push_str(&return_type.format_for_hover());
    }
    signature
}

fn runtime_type_name(generation: GenerationId) -> String {
    format!("RuntimeI64Generation{}", generation.0)
}

fn generation_candidate(candidate: &str, symbol: &str, generation: GenerationId) -> String {
    let runtime_type = runtime_type_name(generation);
    let type_declaration = format!("{runtime_type} is in {RUNTIME_I64_MIN}...{RUNTIME_I64_MAX}\n",);
    let mut generation_source =
        String::with_capacity(type_declaration.len() + candidate.len() + symbol.len() + 20);
    generation_source.push_str(&type_declaration);
    generation_source.push_str(candidate);
    generation_source.push('\n');
    generation_source.push_str(symbol);
    generation_source.push_str("() ");
    generation_source.push_str(&runtime_type);
    generation_source.push_str(": 0\n");
    generation_source
}

fn generation_exports(declared_symbols: &[String], generation: GenerationId) -> Vec<NativeExport> {
    declared_symbols
        .iter()
        .map(|symbol| NativeExport {
            logical_symbol: symbol.clone(),
            generation_symbol: runtime_symbol_name(symbol, generation),
        })
        .collect()
}

fn package_diagnostics(
    package: &CheckedPackage,
    evaluation: Option<&GeneratedEvaluation>,
) -> Vec<InterpreterDiagnostic> {
    let session_path = package.primary_path();
    package
        .files()
        .iter()
        .flat_map(|file| {
            let diagnostics = file
                .front_end_diagnostics()
                .iter()
                .chain(file.typecheck_diagnostics())
                .cloned()
                .collect();
            file_diagnostics(file, diagnostics, session_path, evaluation)
        })
        .collect()
}

fn lowered_diagnostics(
    package: &CheckedPackage,
    diagnostics: Vec<LoweredFileDiagnostics>,
    evaluation: Option<&GeneratedEvaluation>,
) -> Vec<InterpreterDiagnostic> {
    let session_path = package.primary_path();
    diagnostics
        .into_iter()
        .flat_map(|lowered| {
            let file = package
                .file(&lowered.path)
                .expect("lowered diagnostics retain their checked source path");
            file_diagnostics(file, lowered.diagnostics, session_path, evaluation)
        })
        .collect()
}

fn file_diagnostics(
    file: &CheckedFile,
    diagnostics: Vec<Diagnostic>,
    session_path: &std::path::Path,
    evaluation: Option<&GeneratedEvaluation>,
) -> Vec<InterpreterDiagnostic> {
    if file.path() == session_path {
        match evaluation {
            Some(evaluation) => evaluation_diagnostics(evaluation, diagnostics),
            None => submitted_diagnostics(file.source(), diagnostics),
        }
    } else {
        let display_path = file.path().diagnostic_report_path_from_cwd();
        diagnostics
            .into_iter()
            .map(|diagnostic| InterpreterDiagnostic::new(file.source(), &display_path, diagnostic))
            .collect()
    }
}

fn submitted_diagnostics(source: &str, diagnostics: Vec<Diagnostic>) -> Vec<InterpreterDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| submitted_diagnostic(source, diagnostic))
        .collect()
}

fn submitted_diagnostic(source: &str, diagnostic: Diagnostic) -> InterpreterDiagnostic {
    InterpreterDiagnostic::new(source, "<interpreter>", diagnostic)
}

fn evaluation_diagnostics(
    evaluation: &GeneratedEvaluation,
    diagnostics: Vec<Diagnostic>,
) -> Vec<InterpreterDiagnostic> {
    diagnostics
        .into_iter()
        .map(|diagnostic| evaluation_diagnostic(evaluation, diagnostic))
        .collect()
}

fn evaluation_diagnostic(
    evaluation: &GeneratedEvaluation,
    mut diagnostic: Diagnostic,
) -> InterpreterDiagnostic {
    let range = evaluation.expression_range();
    if diagnostic.span.start() >= range.start && diagnostic.span.end() <= range.end {
        diagnostic.span = Span::new(
            diagnostic.span.start() - range.start,
            diagnostic.span.end() - range.start,
        );
        diagnostic.related.retain_mut(|related| {
            if related.span.start() < range.start || related.span.end() > range.end {
                return false;
            }
            related.span = Span::new(
                related.span.start() - range.start,
                related.span.end() - range.start,
            );
            true
        });
        submitted_diagnostic(evaluation.expression(), diagnostic)
    } else {
        generated_diagnostic(evaluation, diagnostic)
    }
}

fn generated_diagnostic(
    evaluation: &GeneratedEvaluation,
    diagnostic: Diagnostic,
) -> InterpreterDiagnostic {
    InterpreterDiagnostic::new(evaluation.source(), "<interpreter-generated>", diagnostic)
}

fn jit_diagnostic(evaluation: &GeneratedEvaluation, error: JitError) -> InterpreterDiagnostic {
    match error {
        JitError::InvalidExportAbi { actual, .. } => submitted_diagnostic(
            evaluation.expression(),
            Diagnostic::new(
                "interpreter-unsupported-result-type",
                format!("expression result cannot be displayed as an integer: found `{actual}`"),
            )
            .with_arg("actual", actual)
            .at_span(Span::new(0, evaluation.expression().len())),
        ),
        error => generated_diagnostic(
            evaluation,
            Diagnostic::new(
                "interpreter-runtime-failure",
                format!("native evaluation failed: {error}"),
            ),
        ),
    }
}

fn codegen_failure(message: &str) -> Diagnostic {
    Diagnostic::new("interpreter-codegen-failure", message)
}

fn has_flaws(diagnostics: &[InterpreterDiagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.diagnostic().category == Category::Flaw)
}

fn compiler_has_flaws(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.category == Category::Flaw)
}

fn evaluation_result_type(checked_file: &CheckedFile) -> Option<ast::ty::Ty> {
    let typed = checked_file.typed();
    let expression = typed.root_exprs.last()?;
    typed.exprs.ty.get(expression.as_usize()).cloned()
}

fn invalid_evaluation_result_type(
    checked_file: &CheckedFile,
    expression: &str,
) -> Option<Diagnostic> {
    let result_type = evaluation_result_type(checked_file)?;

    let registry = &checked_file.typed().type_registry;
    let width = registry.integer_width_for_type(&result_type);
    let interpretation = registry
        .integer_operation_interpretation_for_type(&result_type)
        .or_else(|| result_type.anonymous_operation_interpretation());
    if width == Some(64) && interpretation == Some(IntegerInterpretation::Signed) {
        return None;
    }

    Some(
        Diagnostic::new(
            "interpreter-unsupported-result-type",
            format!(
                "expression result cannot be displayed as an integer: found `{}`",
                result_type.format_for_hover()
            ),
        )
        .with_arg("actual", result_type.format_for_hover())
        .at_span(Span::new(0, expression.len())),
    )
}

#[cfg(test)]
#[path = "tests/session_tests.rs"]
mod tests;
