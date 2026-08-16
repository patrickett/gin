use super::*;
use std::time::{Duration, Instant};
use test_fixtures::TempPackage;

const PACKAGE_NUMBERS: &str = r#"#default(IntegerLiteral)
Signed64 is in -9223372036854775808...9223372036854775807
Small is in 0...255

#intrinsic(BitsAdd)
signed64_add_bits(eat lhs Signed64, eat rhs Signed64) Signed64 extern
#operator(Add)
signed64_add(lhs Signed64, rhs Signed64) Signed64: signed64_add_bits(eat lhs, eat rhs)
"#;

struct NativeSession {
    session: InterpreterSession,
}

struct PackageSession {
    _package: TempPackage,
    session: InterpreterSession,
}

impl NativeSession {
    fn new() -> Self {
        let mut session = InterpreterSession::default();
        let submit = session.submit(
            r#"#default(IntegerLiteral)
Signed64 is in -9223372036854775808...9223372036854775807
Small is in 0...255

#intrinsic(BitsAdd)
signed64_add_bits(eat lhs Signed64, eat rhs Signed64) Signed64 extern
#operator(Add)
#inline
signed64_add(lhs Signed64, rhs Signed64) Signed64: signed64_add_bits(eat lhs, eat rhs)
"#,
        );
        match submit {
            Submission::DeclarationAccepted { .. } => {}
            Submission::Rejected { diagnostics } => {
                println!("bootstrap declaration rejected: {diagnostics:#?}");
                panic!("native bootstrap declaration should be accepted");
            }
            other => panic!("unexpected bootstrap submission result: {other:?}"),
        }
        Self { session }
    }
}

impl PackageSession {
    fn new() -> Self {
        let package = TempPackage::new("interpreter_native_package");
        package.write_flask("interpreter_native_package");
        package.write("numbers/defs.gin", PACKAGE_NUMBERS);
        let context = InterpreterContext::load(package.path()).expect("package context");
        let mut session = InterpreterSession::with_context(context);
        let imported =
            session.submit("use 'numbers'.(Signed64, Small, signed64_add_bits, signed64_add)");
        assert!(
            matches!(imported, Submission::DeclarationAccepted { .. }),
            "package imports should be accepted: {imported:?}"
        );
        Self {
            _package: package,
            session,
        }
    }
}

#[test]
fn native_integer_expression_evaluates() {
    let mut native = NativeSession::new();
    let submission = native.session.submit("40 + 2");

    assert!(
        matches!(
            submission,
            Submission::Evaluated {
                value: RuntimeValue {
                    storage: RuntimeStorage::Int64(42),
                    ..
                },
                ..
            }
        ),
        "unexpected submission: {submission:?}"
    );
}

#[test]
fn expression_can_use_prior_zero_parameter_function() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("a: 1 + 1"),
        Submission::DeclarationAccepted { .. }
    ));

    let submission = native.session.submit("3 + a");

    assert!(
        matches!(
            &submission,
            Submission::Evaluated {
                value: RuntimeValue {
                    storage: RuntimeStorage::Int64(5),
                    ..
                },
                ..
            }
        ),
        "{submission:#?}"
    );
}

#[test]
fn package_callable_executes_in_combined_native_module() {
    let mut package = PackageSession::new();
    let declarations = package.session.declarations().to_string();
    assert!(!declarations.contains("signed64_add(lhs"));

    let submission = package.session.submit("40 + 2");
    assert!(
        matches!(
            submission,
            Submission::Evaluated {
                value: RuntimeValue {
                    storage: RuntimeStorage::Int64(42),
                    ..
                },
                ..
            }
        ),
        "unexpected submission: {submission:#?}"
    );
    assert_eq!(package.session.declarations(), declarations);
}

#[test]
fn expression_can_reference_prior_declaration() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));

    assert!(matches!(
        native.session.submit("value + 2"),
        Submission::Evaluated {
            value: RuntimeValue {
                storage: RuntimeStorage::Int64(42),
                ..
            },
            ..
        }
    ));
}

#[test]
fn expression_uses_latest_redefinition_slot() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    assert!(matches!(
        native.session.submit("value Signed64 := 41"),
        Submission::DeclarationAccepted { .. }
    ));

    assert!(matches!(
        native.session.submit("value + 1"),
        Submission::Evaluated {
            value: RuntimeValue {
                storage: RuntimeStorage::Int64(42),
                ..
            },
            ..
        }
    ));
}

#[test]
fn declaration_generates_live_runtime_entry() {
    let mut native = NativeSession::new();
    let before = native.session.live_runtime().generations().len();
    let submit = native.session.submit("value Signed64 := 40");

    assert!(matches!(submit, Submission::DeclarationAccepted { .. }));
    assert_eq!(
        native.session.live_runtime().generations().len(),
        before + 1
    );
    assert_eq!(
        native
            .session
            .live_runtime()
            .generation_for_symbol("value")
            .map(|id| id.0),
        Some(before as u64 + 1),
    );
}

#[test]
fn redeclaration_updates_live_generation_slot() {
    let mut native = NativeSession::new();

    let first_submit = native.session.submit("value Signed64 := 40");
    assert!(matches!(
        first_submit,
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.live_runtime().generations().len();
    let first_slot = native
        .session
        .live_runtime()
        .generation_for_symbol("value")
        .expect("value is live");

    let second_submit = native.session.submit("value Signed64 := 41");
    assert!(matches!(
        second_submit,
        Submission::DeclarationAccepted { .. }
    ));
    let second_slot = native
        .session
        .live_runtime()
        .generation_for_symbol("value")
        .expect("value is still live");

    assert_eq!(second_slot.0, first_slot.0 + 1);
    assert_eq!(native.session.live_runtime().generations().len(), before,);
}

#[test]
fn active_generation_is_retained_during_active_call_and_retired_after_completion() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.live_runtime().generations().len();

    let active_call = {
        let symbols = native.session.live_runtime().symbols().clone();
        native
            .session
            .live_runtime_mut()
            .track_active_generations_for_symbols(&symbols)
    };

    assert!(matches!(
        native.session.submit("value Signed64 := 41"),
        Submission::DeclarationAccepted { .. }
    ));
    assert_eq!(
        native.session.live_runtime().generations().len(),
        before + 1,
    );

    native
        .session
        .live_runtime_mut()
        .finish_active_generations(active_call);
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn failed_declaration_does_not_change_live_runtime() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));

    let before_len = native.session.live_runtime().generations().len();

    assert!(matches!(
        native.session.submit("value Signed64 := @"),
        Submission::Rejected { .. }
    ));
    assert_eq!(
        native.session.live_runtime().generations().len(),
        before_len
    );

    let value_slot = native
        .session
        .live_runtime()
        .generation_for_symbol("value")
        .expect("previous generation should still be active");
    assert_eq!(value_slot.0, before_len as u64);
}

#[test]
fn clear_disposes_live_generations() {
    let mut native = NativeSession::new();

    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    assert!(!native.session.live_runtime().generations().is_empty());

    assert!(matches!(
        native.session.submit("#clear"),
        Submission::Command(Command::Clear)
    ));
    assert!(native.session.live_runtime().generations().is_empty());
    assert!(native.session.live_runtime().symbols().is_empty());
}

#[test]
fn failed_redeclaration_keeps_previous_live_generation() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.live_runtime().generations().len();
    let first = native
        .session
        .live_runtime()
        .generation_for_symbol("value")
        .expect("value should have a generation");

    assert!(matches!(
        native.session.submit("value Signed64 := missing"),
        Submission::Rejected { .. }
    ));
    assert_eq!(
        native.session.live_runtime().generation_for_symbol("value"),
        Some(first),
    );
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn redeclaration_updates_live_runtime_slot() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.live_runtime().generations().len();

    assert!(matches!(
        native.session.submit("value Signed64 := 41"),
        Submission::DeclarationAccepted { .. }
    ));
    assert_eq!(
        native.session.live_runtime().generation_for_symbol("value"),
        Some(GenerationId(before as u64 + 1)),
    );
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn redeclaration_of_incompatible_scalar_type_is_rejected() {
    let mut package = PackageSession::new();
    assert!(matches!(
        package.session.submit("value Small := 1"),
        Submission::DeclarationAccepted { .. }
    ));

    let response = package.session.submit("value Signed64 := 2");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-incompatible-redefinition"),
                "diagnostics: {diagnostics:?}"
            );
        }
        other => panic!("expected incompatible redefinition to fail, got: {other:?}"),
    }
    assert_eq!(
        package
            .session
            .live_runtime()
            .value_for_symbol("value")
            .unwrap()
            .ty,
        RuntimeType { size: 1, align: 1 },
    );
}

#[test]
fn function_redeclaration_with_compatible_signature_updates_live_generation() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("double(x Signed64) Signed64: x"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.live_runtime().generations().len();

    assert!(matches!(
        native.session.submit("double(x Signed64) Signed64: x"),
        Submission::DeclarationAccepted { .. }
    ));
    assert_eq!(
        native
            .session
            .live_runtime()
            .generation_for_symbol("double")
            .map(|id| id.0),
        Some((before as u64) + 1),
    );
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn function_redefinition_rejects_incompatible_param_count() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("double(x Signed64) Signed64: x"),
        Submission::DeclarationAccepted { .. }
    ));

    let before = native.session.live_runtime().generations().len();
    let response = native
        .session
        .submit("double(x Signed64, y Signed64) Signed64: x + y");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-incompatible-redefinition"),
                "diagnostics: {diagnostics:?}"
            );
        }
        other => panic!("expected incompatible redefinition to fail, got: {other:?}"),
    }
    assert_eq!(
        native
            .session
            .live_runtime()
            .generation_for_symbol("double"),
        Some(GenerationId(before as u64)),
    );
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn function_redefinition_rejects_incompatible_param_convention() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native
            .session
            .submit("double(x Signed64) Signed64:\n    return x"),
        Submission::DeclarationAccepted { .. }
    ));

    let response = native
        .session
        .submit("double(ref x Signed64) Signed64:\n    return x");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-incompatible-redefinition"),
                "diagnostics: {diagnostics:?}"
            );
        }
        other => panic!("expected incompatible redefinition to fail, got: {other:?}"),
    }
}

#[test]
fn function_redefinition_rejects_value_kind_mismatch() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native
            .session
            .submit("double(x Signed64) Signed64:\n    return x"),
        Submission::DeclarationAccepted { .. }
    ));
    let response = native.session.submit("double Signed64 := 12");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-incompatible-redefinition"),
                "diagnostics: {diagnostics:?}"
            );
        }
        other => panic!("expected incompatible redefinition to fail, got: {other:?}"),
    }
}

#[test]
fn declaration_stores_runtime_value_for_scalar_type() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    let value = native
        .session
        .live_runtime()
        .value_for_symbol("value")
        .expect("value has runtime metadata");
    assert_eq!(value.ty, RuntimeType::I64);
}

#[test]
fn expression_wrapper_does_not_persist() {
    let mut native = NativeSession::new();
    let before = native.session.declarations().to_string();

    assert!(matches!(
        native.session.submit("40 + 2"),
        Submission::Evaluated { .. }
    ));
    assert_eq!(native.session.declarations(), before);
    assert!(
        !native
            .session
            .declarations()
            .contains("__gin_interpreter_eval_")
    );
}

#[test]
fn failed_submissions_preserve_declarations() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.declarations().to_string();

    assert!(matches!(
        native.session.submit("missing + 2"),
        Submission::Rejected { .. }
    ));
    assert_eq!(native.session.declarations(), before);
    assert!(matches!(
        native.session.submit("broken Signed64 := missing"),
        Submission::Rejected { .. }
    ));
    assert_eq!(native.session.declarations(), before);
}

#[test]
fn non_i64_expression_is_rejected() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("small Small := 7"),
        Submission::DeclarationAccepted { .. }
    ));
    let before = native.session.declarations().to_string();

    let response = native.session.submit("small");
    if let Submission::Rejected { diagnostics, .. } = &response {
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.diagnostic().code.slug() == "interpreter-unsupported-result-type"
            }),
            "diagnostics: {diagnostics:?}",
        );
    } else {
        panic!("unexpected response: {response:?}");
    }
    assert_eq!(native.session.declarations(), before);
}

#[test]
#[ignore = "debug"]
fn debug_evaluation_candidate_metadata() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("small Small := 7"),
        Submission::DeclarationAccepted { .. }
    ));
    let declarations = native.session.declarations().to_string();
    let checked = native.session.check(&declarations);
    let typed = checked.primary_file().typed();
    let definitions = typed.defs.len();
    println!("definitions: {definitions}");
    for submission in ["40 + 2", "small"] {
        let generation = if submission == "40 + 2" { 1 } else { 2 };
        let evaluation = native.session.source.evaluation_candidate_with_dispatches(
            submission,
            generation,
            native.session.live_runtime.symbols(),
        );
        println!("evaluation source:\n{}", evaluation.source());

        let checked = native.session.check(evaluation.source());
        println!(
            "front_end_flaws={} typecheck_flaws={}",
            checked.has_front_end_flaws(),
            checked.has_typecheck_flaws(),
        );

        let diagnostics = package_diagnostics(&checked, Some(&evaluation));
        println!("package diagnostics: {}", diagnostics.len());
        for diagnostic in diagnostics {
            println!(
                "diagnostic={} source={:?} path={}",
                diagnostic.diagnostic().message,
                diagnostic.source(),
                diagnostic.display_path()
            );
        }

        let typed = checked.primary_file().typed();
        for root_expr in typed.root_exprs.iter() {
            let expr = typed.expr(*root_expr).expect("root expr exists");
            println!("root expr {} type={:?}", root_expr.as_usize(), expr.ty);
        }
        for bind in typed.defs.values() {
            if bind.name.as_str().contains("__gin_interpreter_eval_") {
                println!(
                    "eval bind {} return_type={:?} declared_return_type={:?}",
                    bind.name.as_str(),
                    bind.return_type,
                    bind.declared_return_type,
                );
            }
        }

        let context = NativeCompiler::create_context();
        let lowered = native.session.driver.lower_package(&context, &checked);
        println!("lowered modules: {}", lowered.diagnostics.len());
        match lowered.module {
            Some(module) => {
                println!("lowered module:\n{:?}", module);
            }
            None => {
                println!("lowered module missing");
            }
        }

        let typed = checked.primary_file().typed();
        for bind in typed.defs.values() {
            if bind.name.as_str().starts_with("__gin_interpreter_eval_") {
                println!(
                    "definition {} return_type={:?} declared_return_type={:?}",
                    bind.name.as_str(),
                    bind.return_type,
                    bind.declared_return_type,
                );
            }
        }
    }
}

#[test]
fn function_header_requests_more_input() {
    let mut session = InterpreterSession::default();

    assert!(matches!(
        session.submit("double(value Signed64) Signed64:"),
        Submission::Incomplete
    ));
    assert!(session.declarations().is_empty());
}

#[test]
fn multiline_function_is_accepted_atomically() {
    let mut native = NativeSession::new();

    assert!(matches!(
        native
            .session
            .submit("forty_two() Signed64:\n    result Signed64 := 42\nreturn result"),
        Submission::DeclarationAccepted { .. }
    ));
    assert!(native.session.declarations().contains("forty_two()"));
}

#[test]
fn mixed_submissions_are_rejected_without_changing_source() {
    let mut native = NativeSession::new();
    let before = native.session.declarations().to_string();

    assert!(matches!(
        native.session.submit("answer Signed64 := 42\nanswer"),
        Submission::Rejected { diagnostics, .. }
            if diagnostics[0].diagnostic().code.slug() == "interpreter-mixed-submission"
    ));
    assert_eq!(native.session.declarations(), before);
}

#[test]
fn clear_discards_accumulated_source() {
    let mut native = NativeSession::new();

    assert!(matches!(
        native.session.submit("#clear"),
        Submission::Command(Command::Clear)
    ));
    assert!(native.session.declarations().is_empty());
}

#[test]
fn exit_is_a_quit_alias() {
    let mut session = InterpreterSession::default();

    assert!(matches!(
        session.submit("#exit"),
        Submission::Command(Command::Quit)
    ));
}

#[test]
fn compiler_attributes_are_not_interpreter_commands() {
    let mut session = InterpreterSession::default();

    assert!(!matches!(
        session.submit("#wat"),
        Submission::UnknownCommand(_)
    ));
}

#[test]
fn run_command_starts_background_task() {
    let mut native = NativeSession::new();

    let task_id = match native.session.submit("#run 40 + 2") {
        Submission::Command(Command::TaskStarted(task_id)) => task_id,
        other => panic!("run command should start a task, got {other:?}"),
    };

    let summaries = native.session.task_summaries();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].id, task_id);
    assert_eq!(summaries[0].expression, "40 + 2");
}

#[test]
fn run_command_can_request_task_stop() {
    let mut native = NativeSession::new();

    let task_id = match native.session.submit("#run 40 + 2") {
        Submission::Command(Command::TaskStarted(task_id)) => task_id,
        other => panic!("run command should start a task, got {other:?}"),
    };

    assert!(matches!(
        native.session.submit(&format!("#stop {}", task_id.0)),
        Submission::Command(Command::TaskStopRequested(id))
        if id == task_id
    ));
}

#[test]
fn run_command_rejects_stop_for_unknown_task_id() {
    let mut native = NativeSession::new();

    let response = native.session.submit("#stop 99999");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-invalid-task-id")
            );
        }
        other => panic!("stop should fail for unknown id, got: {other:?}"),
    }
}

#[test]
fn run_command_requires_expression_argument() {
    let mut native = NativeSession::new();

    let response = native.session.submit("#run");
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug()
                        == "interpreter-invalid-command-usage")
            );
        }
        other => panic!("run without args should fail, got: {other:?}"),
    }
}

#[test]
fn run_results_are_polled_to_completion() {
    let mut native = NativeSession::new();

    let started = match native.session.submit("#run 40 + 2") {
        Submission::Command(Command::TaskStarted(task_id)) => task_id,
        other => panic!("run command should start a task, got {other:?}"),
    };

    let timeout = Instant::now() + Duration::from_secs(2);
    let mut result = None;
    while Instant::now() < timeout {
        for task_result in native.session.poll_task_results() {
            if task_result.id == started {
                match task_result.submission {
                    Submission::Evaluated { .. } => result = Some(task_result),
                    Submission::Rejected { .. } => {
                        panic!("run task should evaluate successfully");
                    }
                    _ => {}
                }
            }
        }
        if result.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let Some(task_result) = result else {
        panic!("expected task result before timeout");
    };
    assert_eq!(task_result.id, started);
    assert!(matches!(task_result.state, TaskState::Completed));
}

#[test]
fn reset_clears_running_tasks_and_runtime_state() {
    let mut native = NativeSession::new();

    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));
    assert!(!native.session.live_runtime().generations().is_empty());

    let _started = match native.session.submit("#run value + 2") {
        Submission::Command(Command::TaskStarted(task_id)) => task_id,
        other => panic!("run command should start a task, got {other:?}"),
    };
    assert!(!native.session.task_summaries().is_empty());

    assert!(matches!(
        native.session.submit("#reset"),
        Submission::Command(Command::Reset)
    ));
    assert!(native.session.task_summaries().is_empty());
    assert!(native.session.live_runtime().generations().is_empty());
    assert!(native.session.live_runtime().symbols().is_empty());
    assert!(native.session.live_runtime().values().is_empty());
    assert!(!native.session.declarations().contains("value Signed64"));
}

#[test]
fn parse_error_after_declaration_owns_raw_submission_source() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("value Signed64 := 40"),
        Submission::DeclarationAccepted { .. }
    ));

    let Submission::Rejected { diagnostics } = native.session.submit("broken := @") else {
        panic!("invalid declaration should be rejected");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.diagnostic().code.slug().starts_with("parse-"))
        .expect("parse diagnostic");

    assert_eq!(diagnostic.source(), "broken := @");
    assert_eq!(diagnostic.display_path(), "<interpreter>");
    assert!(diagnostic.diagnostic().span.end() <= diagnostic.source().len());
}

#[test]
fn package_diagnostic_owns_package_source_and_path() {
    let package = TempPackage::new("diagnostic_package");
    package.write_flask("diagnostic_package");
    let package_source = "broken := @\n";
    package.write("broken.gin", package_source);
    let context = InterpreterContext::load(package.path()).expect("package context");
    let mut session = InterpreterSession::with_context(context);

    let Submission::Rejected { diagnostics } = session.submit("SessionTag is Unit") else {
        panic!("package flaw should reject the submission");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.display_path().ends_with("broken.gin"))
        .expect("package diagnostic");

    assert_eq!(diagnostic.source(), package_source);
    assert_ne!(diagnostic.display_path(), "<interpreter>");
    assert_ne!(diagnostic.display_path(), "<interpreter-generated>");
}

#[test]
fn reserved_session_file_is_excluded_from_package_sources() {
    let package = TempPackage::new("reserved_session_file");
    package.write_flask("reserved_session_file");
    package.write(".gin-interpreter.gin", "broken := @\n");
    let context = InterpreterContext::load(package.path()).expect("package context");

    assert!(
        context
            .package()
            .sources()
            .iter()
            .all(|source| source.path != context.package().session_path())
    );

    let mut session = InterpreterSession::with_context(context);
    assert!(matches!(
        session.submit("SessionTag is Unit"),
        Submission::DeclarationAccepted { .. }
    ));
    assert_eq!(session.declarations(), "SessionTag is Unit\n");
}

#[test]
fn wrapper_type_error_maps_to_user_expression() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("small Small := 7"),
        Submission::DeclarationAccepted { .. }
    ));

    let Submission::Rejected { diagnostics } = native.session.submit("small") else {
        panic!("non-i64 expression should be rejected");
    };
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.diagnostic().code.slug() == "interpreter-unsupported-result-type"
        })
        .expect("unsupported result diagnostic");

    assert_eq!(diagnostic.source(), "small");
    assert_eq!(diagnostic.display_path(), "<interpreter>");
    assert!(!diagnostic.source().contains("__gin_interpreter_eval_"));
    assert!(
        !diagnostic
            .display_path()
            .contains("__gin_interpreter_eval_")
    );
    assert!(
        !diagnostic
            .diagnostic()
            .message
            .contains("__gin_interpreter_eval_")
    );
}
