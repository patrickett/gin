use crate::runtime::{
    GenerationId, LiveRuntime, NativeExport, NativeGeneration, RuntimeDeclarationKind,
    RuntimeFunction, RuntimeStorage, RuntimeType, RuntimeValue, generation_wrapper_name,
    runtime_symbol_name,
};
use crate::source::RuntimeDeclaration;
use crate::{
    GeneratedEvaluation, InterpreterCondition, InterpreterContext, InterpreterDiagnostic,
    InterpreterFrame, SessionSource,
};
use ast_format::declare::BindFormatExt;
use codegen::{NativeCompiler, NativeEngine};
use diagnostic::code::{EvaluationCode, InterpreterCode};
use diagnostic::{Category, Diagnostic, DiagnosticCode, DiagnosticPathExt, Span};
use ginc::driver::{CheckedFile, CheckedPackage, CompilerDriver, LoweredFileDiagnostics};
use parser::query::SourceParseExt;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
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
        value: crate::value::EvaluatedValue,
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    Inspected {
        value: crate::value::EvaluatedValue,
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    Answered {
        text: String,
    },
    Command(Command),
    Empty,
    Incomplete,
    Rejected {
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    Cancelled {
        diagnostics: Vec<InterpreterDiagnostic>,
    },
    UnknownCommand(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Retry,
    Abort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeEntry {
    pub name: String,
    pub signature: String,
}

#[derive(Debug)]
pub enum QueryOutcome {
    Answered(String),
    Rejected(Vec<InterpreterDiagnostic>),
    Empty,
}

pub struct InterpreterSession {
    context: InterpreterContext,
    source: SessionSource,
    image: Option<queries::AcceptedImage>,
    package_image: std::cell::OnceCell<queries::AcceptedImage>,
    next_generation: u64,
    next_runtime_generation: u64,
    next_task_id: u64,
    live_runtime: LiveRuntime,
    running_tasks: HashMap<TaskId, SpawnedTask>,
    driver: CompilerDriver,
    eval_state: Arc<Mutex<typecheck::EvalState>>,
    pending: Option<InterpreterCondition>,
    task_conditions: HashMap<TaskId, InterpreterCondition>,
}

const RUNTIME_I64_MIN: &str = "-9223372036854775808";
const RUNTIME_I64_MAX: &str = "9223372036854775807";
impl Default for InterpreterSession {
    fn default() -> Self {
        Self::with_context(InterpreterContext::isolated())
    }
}

impl InterpreterSession {
    pub fn with_context(context: InterpreterContext) -> Self {
        Self {
            context,
            source: SessionSource::default(),
            image: None,
            package_image: std::cell::OnceCell::new(),
            next_generation: 1,
            next_runtime_generation: 1,
            next_task_id: 1,
            running_tasks: HashMap::new(),
            live_runtime: LiveRuntime::default(),
            driver: CompilerDriver::default(),
            eval_state: Arc::new(Mutex::new(typecheck::EvalState::default())),
            pending: None,
            task_conditions: HashMap::new(),
        }
    }

    pub fn context(&self) -> &InterpreterContext {
        &self.context
    }

    pub fn declarations(&self) -> &str {
        self.source.declarations()
    }

    pub fn pending(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(|condition| condition.submission.as_str())
    }

    pub fn condition(&self) -> Option<&InterpreterCondition> {
        self.pending.as_ref()
    }

    pub fn frames(&self) -> &[InterpreterFrame] {
        self.condition()
            .map(InterpreterCondition::frames)
            .unwrap_or_default()
    }

    pub fn frame(&self, index: usize) -> Option<&InterpreterFrame> {
        self.frames().get(index)
    }

    pub fn task_condition(&self, id: TaskId) -> Option<&InterpreterCondition> {
        self.task_conditions.get(&id)
    }

    pub fn retry_task(&mut self, id: TaskId) -> Option<Submission> {
        let expression = self.task_conditions.get(&id)?.submission.clone();
        let task = self.spawn_task(&expression);
        self.task_conditions.remove(&id);
        Some(Submission::Command(Command::TaskStarted(task)))
    }

    pub fn abort_task(&mut self, id: TaskId) -> bool {
        self.task_conditions.remove(&id).is_some()
    }

    pub fn restarts(&self) -> &'static [Restart] {
        self.condition()
            .map(InterpreterCondition::restarts)
            .unwrap_or_default()
    }

    pub fn retry(&mut self) -> Option<Submission> {
        let pending = self.pending()?.to_string();
        let result = self.dispatch(&pending);
        self.pending = result.condition(&pending);
        Some(result)
    }

    pub fn abort(&mut self) -> bool {
        self.pending.take().is_some()
    }

    pub fn materialize(&mut self) -> Result<&str, Vec<InterpreterDiagnostic>> {
        let source = self.source.declarations().to_string();
        let context = self.context.reload().map_err(|error| {
            vec![submitted_diagnostic(
                &source,
                // interpreter-materialization-context-unavailable
                Diagnostic::new(
                    InterpreterCode::MaterializationContextUnavailable,
                    error.to_string(),
                ),
            )]
        })?;
        let checked = context
            .check_source(
                &mut CompilerDriver::default(),
                &source,
                context.package().sources().to_vec(),
            )
            .expect("interpreter includes its primary session source");
        let mut diagnostics = package_diagnostics(&checked, None);
        diagnostics
            .retain(|diagnostic| diagnostic.diagnostic().code != DiagnosticCode::UnfilledHole);
        for file in checked.files() {
            diagnostics.extend(file_diagnostics(
                file,
                file.hole_diagnostics(),
                checked.primary_path(),
                None,
            ));
        }
        if has_flaws(&diagnostics) {
            Err(diagnostics)
        } else {
            Ok(self.source.declarations())
        }
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
                    completed.push(result);
                    done.push(task_id);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    completed.push(TaskResult {
                        id: task_id,
                        expression: task.expression.clone(),
                        state: TaskState::Failed,
                        submission: Submission::Rejected {
                            diagnostics: vec![submitted_diagnostic(
                                "",
                                // interpreter-task-failure
                                Diagnostic::new(
                                    InterpreterCode::TaskFailure,
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

        for result in &completed {
            if let Some(condition) = result.condition() {
                self.task_conditions.insert(result.id, condition);
            }
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
            &HashMap::new(),
        );
        let context = self.context.clone();
        let eval_state = self.eval_state.clone();
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
            let result = run_task_submission(context, evaluation, task_id, stop_token, eval_state);
            let _ = sender.send(result);
        });

        self.running_tasks.insert(
            id,
            SpawnedTask {
                expression: expression.to_string(),
                stop_requested: stop,
                receiver,
            },
        );

        id
    }

    pub fn submit(&mut self, input: &str) -> Submission {
        if input.trim() == "#retry" {
            return self.retry().unwrap_or_else(|| Submission::Answered {
                text: "no pending condition".to_string(),
            });
        }
        let result = self.dispatch(input);
        if !queries::is_query_command(input)
            && let Some(condition) = result.condition(input.trim_end())
        {
            self.pending = Some(condition);
        }
        result
    }

    fn dispatch(&mut self, input: &str) -> Submission {
        let input = input.trim_end();
        if input.trim().is_empty() {
            return Submission::Empty;
        }

        if let Some(command) = input.trim().strip_prefix('#') {
            let name = command.split_whitespace().next();
            if matches!(
                name,
                Some(
                    "clear"
                        | "reset"
                        | "tasks"
                        | "stop"
                        | "run"
                        | "type"
                        | "inspect"
                        | "scope"
                        | "doc"
                        | "search"
                        | "complete"
                        | "queries"
                        | "suggest"
                        | "frames"
                        | "frame"
                        | "retry"
                        | "abort"
                        | "save"
                        | "help"
                        | "exit"
                        | "quit"
                        | "q"
                )
            ) {
                return self.command(command);
            }
        }

        let parsed = input.parse_source_full();
        if parsed.is_incomplete() {
            return Submission::Incomplete;
        }
        if crate::submission_syntax::is_standalone_expression(input, &parsed) {
            return self.evaluate(input);
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
        if self.context.is_isolated() && !parsed.ast.uses.is_empty() {
            return Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    input,
                    // interpreter-import-requires-package
                    Diagnostic::new(
                        InterpreterCode::ImportRequiresPackage,
                        "imports require a package context",
                    )
                    .at_span(Span::new(0, input.len()))
                    .with_help("start in a package directory or pass its path to ginterpreter"),
                )],
            };
        }

        match SubmissionKind::from_ast(&parsed.ast) {
            SubmissionKind::Mixed => Submission::Rejected {
                diagnostics: submitted_diagnostics(
                    input,
                    // interpreter-mixed-submission
                    vec![Diagnostic::new(
                        InterpreterCode::MixedSubmission,
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
        let mut result = self.submit_declaration_checked(input, &candidate, declaration_names, ast);
        let diagnostics = match &mut result {
            Submission::DeclarationAccepted { diagnostics }
            | Submission::Rejected { diagnostics }
            | Submission::Cancelled { diagnostics } => diagnostics,
            _ => return result,
        };
        let start = candidate.len() - input.len() - 1;
        assert_eq!(&candidate[start..candidate.len() - 1], input);
        for diagnostic in diagnostics {
            diagnostic.rebase_submission(&candidate, input, start);
        }
        result
    }

    fn submit_declaration_checked(
        &mut self,
        input: &str,
        candidate: &str,
        declaration_names: Vec<String>,
        ast: &ast::FileAst,
    ) -> Submission {
        if let Some(help) = self.source.replacement_conflict(&declaration_names) {
            return Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    input,
                    // interpreter-shared-declaration
                    Diagnostic::new(
                        InterpreterCode::SharedDeclaration,
                        "replacement would remove other names from a shared declaration",
                    )
                    .at_span(Span::new(0, input.len()))
                    .with_help(help),
                )],
            };
        }
        let checked = self.check(candidate);
        let mut diagnostics = package_diagnostics(&checked, None);
        if checked.has_front_end_flaws() || checked.has_typecheck_flaws() {
            return Submission::Rejected { diagnostics };
        }

        let combined_ast = &checked.primary_file().parsed().output.ast;
        let newly_private = ast
            .defs
            .values()
            .map(|bind| bind.name)
            .find(|name| {
                !ast.private_defs.contains(name) && combined_ast.private_defs.contains(name)
            })
            .or_else(|| {
                ast.tags.keys().copied().find(|name| {
                    !ast.private_tags.contains(name) && combined_ast.private_tags.contains(name)
                })
            });
        if let Some(name) = newly_private {
            return Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    input,
                    // interpreter-private-section
                    Diagnostic::new(
                        InterpreterCode::PrivateSection,
                        format!("`{name}` would become private after the session's private section"),
                    )
                    .at_span(Span::new(0, input.len()))
                    .with_help("save the session, place public declarations before `private`, then #clear and resubmit the combined source"),
                )],
            };
        }

        let runtime_metadata = declaration_runtime_metadata(&checked, &declaration_names);
        let mut compatible = true;
        for declaration in &runtime_metadata {
            let Some(existing_value) = self.live_runtime.value_for_symbol(&declaration.name) else {
                continue;
            };
            if !declarations_compatible(existing_value, declaration) {
                diagnostics.push(submitted_diagnostic(
                    input,
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
            self.image = Some(queries::AcceptedImage::new(checked));
            return Submission::DeclarationAccepted { diagnostics };
        }

        if !checked.has_holes() {
            let context = NativeCompiler::create_context();
            let lowered = self.driver.lower_package(&context, &checked);
            diagnostics.extend(lowered_diagnostics(&checked, lowered.diagnostics, None));
            if has_flaws(&diagnostics) || lowered.module.is_none() {
                if lowered.module.is_none() && !has_flaws(&diagnostics) {
                    diagnostics.push(submitted_diagnostic(
                        input,
                        codegen_failure("code generation produced no module"),
                    ));
                }
                return Submission::Rejected { diagnostics };
            }
        }

        let generation_id = GenerationId(self.next_runtime_generation);
        let generation_symbol = generation_wrapper_name(generation_id);
        let generation_candidate =
            generation_candidate(candidate, &generation_symbol, generation_id);
        let runtime_engine = if checked.has_holes() {
            None
        } else {
            self.build_runtime_generation(&generation_candidate, &generation_symbol)
        };

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

        self.image = Some(queries::AcceptedImage::new(checked));
        Submission::DeclarationAccepted { diagnostics }
    }

    fn evaluate(&mut self, input: &str) -> Submission {
        let generation = self.next_generation;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("interpreter evaluation generation overflow");
        let evaluation =
            self.source
                .evaluation_candidate_with_dispatches(input, generation, &HashMap::new());
        let checked = self.check(evaluation.source());
        evaluate_checked_submission(
            &checked,
            &evaluation,
            Arc::new(AtomicBool::new(false)),
            &self.eval_state,
        )
    }

    fn check(&mut self, source: &str) -> CheckedPackage {
        self.context
            .check_source(
                &mut self.driver,
                source,
                self.context.package().sources().to_vec(),
            )
            .expect("interpreter includes its primary session source")
    }

    fn command(&mut self, command: &str) -> Submission {
        let command = command.trim();
        let mut parts = command.splitn(2, char::is_whitespace).map(str::trim);
        match parts.next() {
            Some(action @ ("clear" | "reset")) => {
                let reset = action == "reset";
                self.source.clear();
                self.image = None;
                self.live_runtime.clear();
                self.pending = None;
                self.task_conditions.clear();
                *self.eval_state.lock().expect("evaluator state lock") =
                    typecheck::EvalState::default();
                if reset {
                    self.clear_tasks();
                }
                Submission::Command(if reset {
                    Command::Reset
                } else {
                    Command::Clear
                })
            }
            Some("tasks") => Submission::Command(Command::Tasks),
            Some("stop") => match parts.next().and_then(|id| id.parse::<u64>().ok()) {
                Some(task_id) if self.stop_task(TaskId(task_id)) => {
                    Submission::Command(Command::TaskStopRequested(TaskId(task_id)))
                }
                Some(_) => Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        command,
                        // interpreter-invalid-task-id
                        Diagnostic::new(InterpreterCode::InvalidTaskId, "unknown task id"),
                    )],
                },
                None => Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        command,
                        // interpreter-invalid-command-usage
                        Diagnostic::new(
                            InterpreterCode::InvalidCommandUsage,
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
                        // interpreter-invalid-command-usage
                        Diagnostic::new(
                            InterpreterCode::InvalidCommandUsage,
                            "usage: #run <expression>",
                        ),
                    )],
                },
            },
            Some("type") => match parts
                .next()
                .map(str::trim)
                .filter(|query| !query.is_empty())
            {
                Some(expression) => self.type_submission(expression),
                None => Submission::Answered {
                    text: "usage: #type <expression>".to_string(),
                },
            },
            Some("scope") => Submission::Answered {
                text: self.scope_report(),
            },
            Some("doc") => self.documentation_report(parts.next()),
            Some("inspect") => self.inspect(parts.next().unwrap_or_default()),
            Some("search") => self.search_report(parts.next()),
            Some("complete") => self.completions_report(parts.next()),
            Some("queries") => Submission::Answered {
                text: crate::help::query_help(parts.next()),
            },
            Some("suggest") => self.suggestions_report(parts.next()),
            Some(name @ ("frames" | "frame" | "retry" | "abort")) => {
                self.condition_command(name, parts.next())
            }
            Some("save") => match parts.next().map(str::trim).filter(|path| !path.is_empty()) {
                Some(path) => self.save_report(path),
                None => Submission::Answered {
                    text: "usage: #save <path>".to_string(),
                },
            },
            Some("help") => Submission::Answered {
                text: crate::help(parts.next()),
            },
            Some("exit") | Some("quit") | Some("q") => Submission::Command(Command::Quit),
            Some(other) => Submission::UnknownCommand(other.to_string()),
            None => Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    command,
                    // interpreter-invalid-command
                    Diagnostic::new(InterpreterCode::InvalidCommand, "unknown command"),
                )],
            },
        }
    }

    fn type_submission(&mut self, expression: &str) -> Submission {
        match self.type_of(expression) {
            QueryOutcome::Answered(text) => Submission::Answered { text },
            QueryOutcome::Rejected(diagnostics) => Submission::Rejected { diagnostics },
            QueryOutcome::Empty => Submission::Answered {
                text: "expression has no result type".to_string(),
            },
        }
    }

    fn scope_report(&self) -> String {
        let entries = self.scope();
        if entries.is_empty() {
            return "empty image".to_string();
        }
        entries
            .iter()
            .map(|entry| {
                if entry.name.is_empty() {
                    entry.signature.clone()
                } else {
                    format!("{}: {}", entry.name, entry.signature)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn save_report(&mut self, path: &str) -> Submission {
        let source = match self.materialize() {
            Ok(source) => source,
            Err(diagnostics) => return Submission::Rejected { diagnostics },
        };
        match std::fs::write(path, source) {
            Ok(()) => Submission::Answered {
                text: format!("saved image source to {path}"),
            },
            Err(error) => Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    "#save",
                    // interpreter-save-failed
                    Diagnostic::new(InterpreterCode::SaveFailed, error.to_string()),
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

        let engine = NativeEngine::new(runtime_module, generation_symbol).ok()?;
        Some(engine)
    }
}

fn run_task_submission(
    context: InterpreterContext,
    evaluation: GeneratedEvaluation,
    task_id: TaskId,
    stop_requested: Arc<AtomicBool>,
    eval_state: Arc<Mutex<typecheck::EvalState>>,
) -> TaskResult {
    let worker = std::panic::AssertUnwindSafe(|| {
        run_task_evaluation(&context, &evaluation, stop_requested, &eval_state)
    });
    let submission = match std::panic::catch_unwind(worker) {
        Ok(submission) => submission,
        Err(_) => Submission::Rejected {
            diagnostics: vec![submitted_diagnostic(
                evaluation.expression(),
                // interpreter-task-failed
                Diagnostic::new(InterpreterCode::TaskFailed, "task panicked"),
            )],
        },
    };
    let state = match &submission {
        Submission::Evaluated { .. } => TaskState::Completed,
        Submission::Cancelled { .. } => TaskState::Cancelled,
        _ => TaskState::Failed,
    };
    TaskResult {
        id: task_id,
        expression: evaluation.expression().to_string(),
        state,
        submission,
    }
}

fn run_task_evaluation(
    context: &InterpreterContext,
    evaluation: &GeneratedEvaluation,
    stop_requested: Arc<AtomicBool>,
    eval_state: &Mutex<typecheck::EvalState>,
) -> Submission {
    if stop_requested.load(Ordering::SeqCst) {
        return Submission::Cancelled {
            diagnostics: vec![submitted_diagnostic(
                evaluation.expression(),
                // compile-time-cancelled
                Diagnostic::new(EvaluationCode::Cancelled, "evaluation was cancelled"),
            )],
        };
    }
    let mut driver = CompilerDriver::default();
    let checked = match context.check_source(
        &mut driver,
        evaluation.source(),
        context.package().sources().to_vec(),
    ) {
        Ok(checked) => checked,
        Err(error) => {
            return Submission::Rejected {
                diagnostics: vec![submitted_diagnostic(
                    evaluation.expression(),
                    // interpreter-task-failed
                    Diagnostic::new(InterpreterCode::TaskFailed, error.to_string()),
                )],
            };
        }
    };
    evaluate_checked_submission(&checked, evaluation, stop_requested, eval_state)
}

fn evaluate_checked_submission(
    checked: &CheckedPackage,
    evaluation: &GeneratedEvaluation,
    stop_requested: Arc<AtomicBool>,
    eval_state: &Mutex<typecheck::EvalState>,
) -> Submission {
    let mut diagnostics = package_diagnostics(checked, Some(evaluation));
    if checked.has_front_end_flaws() || checked.has_typecheck_flaws() {
        return Submission::Rejected { diagnostics };
    }
    match crate::execution::execute(checked, evaluation.symbol(), stop_requested, eval_state) {
        crate::execution::Execution::Completed(value) => {
            Submission::Evaluated { value, diagnostics }
        }
        crate::execution::Execution::Failed(failure) => {
            diagnostics.push(execution_failure(checked, evaluation, failure));
            Submission::Rejected { diagnostics }
        }
        crate::execution::Execution::Cancelled(failure) => {
            diagnostics.push(execution_failure(checked, evaluation, failure));
            Submission::Cancelled { diagnostics }
        }
    }
}

fn execution_diagnostic(
    evaluation: &GeneratedEvaluation,
    mut diagnostic: Diagnostic,
) -> InterpreterDiagnostic {
    if diagnostic.span.is_empty() {
        diagnostic.span = Span::new(0, evaluation.expression().len());
        submitted_diagnostic(evaluation.expression(), diagnostic)
    } else {
        evaluation_diagnostic(evaluation, diagnostic)
    }
}

fn execution_failure(
    checked: &CheckedPackage,
    evaluation: &GeneratedEvaluation,
    failure: crate::execution::ExecutionFailure,
) -> InterpreterDiagnostic {
    execution_diagnostic(evaluation, failure.diagnostic).with_frames(
        crate::condition::resolve_frames(checked, Some(evaluation), failure.frames),
    )
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
    names.extend(ast.defs.keys().map(|id| id.0.to_string()));
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
                    param_conventions: declaration.param_conventions.to_vec(),
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
            existing.ty == replacement.return_type
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
        align: u8::try_from(typecheck::layout::scalar_alignment(u64::from(size)))
            .expect("runtime alignment is always small"),
    })
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

    // interpreter-incompatible-redefinition
    Diagnostic::new(
        InterpreterCode::IncompatibleRedefinition,
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

    for bind in ast.defs.values().chain(&ast.method_binds) {
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
                            .unwrap_or(ast::ParamConvention::Terminal)
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
        && matches!(return_type, ast::ty::Ty::Named { .. })
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
    let type_declaration = format!(
        "{runtime_type} is in {RUNTIME_I64_MIN} to {RUNTIME_I64_MAX}\n{runtime_type} has\n    finish(self):\n        return\n"
    );
    let mut generation_source =
        String::with_capacity(type_declaration.len() + candidate.len() + symbol.len() + 20);
    generation_source.push_str(&type_declaration);
    generation_source.push_str(candidate);
    generation_source.push('\n');
    generation_source.push_str(symbol);
    generation_source.push(' ');
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
    package
        .files()
        .iter()
        .flat_map(|file| checked_file_diagnostics(package, file, evaluation))
        .collect()
}

fn checked_file_diagnostics(
    package: &CheckedPackage,
    file: &CheckedFile,
    evaluation: Option<&GeneratedEvaluation>,
) -> Vec<InterpreterDiagnostic> {
    file.front_end_diagnostics()
        .iter()
        .chain(file.typecheck_diagnostics())
        .map(|diagnostic| {
            let frames = file
                .typed()
                .eval_conditions
                .iter()
                .find(|condition| condition.failure.diagnostic == *diagnostic)
                .map(|condition| {
                    crate::condition::resolve_frames(
                        package,
                        evaluation,
                        condition.failure.frames.clone(),
                    )
                })
                .unwrap_or_default();
            file_diagnostics(
                file,
                vec![diagnostic.clone()],
                package.primary_path(),
                evaluation,
            )
            .pop()
            .expect("one diagnostic retains its source")
            .with_frames(frames)
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
        diagnostic.related_mut().retain_mut(|related| {
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

fn codegen_failure(message: &str) -> Diagnostic {
    // interpreter-codegen-failure
    Diagnostic::new(InterpreterCode::CodegenFailure, message)
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

#[cfg(test)]
#[path = "tests/session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/execution_tests.rs"]
mod execution_tests;

#[cfg(test)]
#[path = "tests/condition_tests.rs"]
mod condition_tests;

#[cfg(test)]
#[path = "tests/hole_tests.rs"]
mod hole_tests;

#[cfg(test)]
#[path = "tests/compiler_condition_tests.rs"]
mod compiler_condition_tests;

#[path = "session_actions.rs"]
mod actions;

#[cfg(test)]
#[path = "tests/code_action_tests.rs"]
mod code_action_tests;

#[path = "session_queries.rs"]
mod queries;
pub use queries::{InterpreterCompletion, SearchMatch};

#[path = "session_inspection.rs"]
mod inspection;

#[cfg(test)]
#[path = "tests/query_tests.rs"]
mod query_tests;
