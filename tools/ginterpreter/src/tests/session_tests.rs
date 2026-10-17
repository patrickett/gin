use super::*;
use ast::prelude::Intern;
use std::time::{Duration, Instant};
use test_fixtures::TempPackage;

const PACKAGE_NUMBERS: &str = r#"Signed64 is in -9223372036854775808 to 9223372036854775807
Signed64 has
    finish(self):
        return
Small is in 0 to 255
Small has
    finish(self):
        return

signed64_add_bits(lhs Signed64, rhs Signed64) Signed64:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Signed64
#infix('+')
signed64_add(lhs Signed64, rhs Signed64) Signed64: return signed64_add_bits(lhs, rhs)
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
            r#"Signed64 is in -9223372036854775808 to 9223372036854775807
Signed64 has
    finish(self):
        return
Small is in 0 to 255
Small has
    finish(self):
        return

signed64_add_bits(lhs Signed64, rhs Signed64) Signed64:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Signed64
#infix('+')
#inline
signed64_add(lhs Signed64, rhs Signed64) Signed64: return signed64_add_bits(lhs, rhs)
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
            session.submit("use 'numbers' Signed64, Small, signed64_add_bits, signed64_add");
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

fn declared(native: &mut NativeSession, src: &str) {
    assert!(
        matches!(
            native.session.submit(src),
            Submission::DeclarationAccepted { .. }
        ),
        "declaration should be accepted: {src}"
    );
}

fn eval_int(native: &mut NativeSession, src: &str, expected: i64) {
    let submission = native.session.submit(src);
    assert!(
        matches!(
            submission,
            Submission::Evaluated {
                value: crate::value::EvaluatedValue {
                    storage: RuntimeStorage::Int64(value),
                    ..
                },
                ..
            } if value == expected
        ),
        "unexpected submission: {submission:?}"
    );
}

fn rejected_with_code(response: Submission, code: &str) {
    match response {
        Submission::Rejected { diagnostics } => {
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.diagnostic().code.slug() == code),
                "expected `{code}`, diagnostics: {diagnostics:?}"
            );
        }
        other => panic!("expected `{code}` rejection, got: {other:?}"),
    }
}

#[test]
fn typed_integer_expression_evaluates() {
    let mut native = NativeSession::new();
    eval_int(&mut native, "40 + 2", 42);
}

#[test]
fn nominal_signed_addition_can_choose_explicit_wrapping() {
    let mut native = NativeSession::new();
    for (left, right, expected) in [
        (i64::MAX, 1, i64::MIN),
        (i64::MIN, -1, i64::MAX),
        (-40, -2, -42),
        (40, 2, 42),
    ] {
        eval_int(
            &mut native,
            &format!("signed64_add_bits({left}, {right})"),
            expected,
        );
    }
}

#[test]
fn ordinary_double_quoted_text_uses_byte_array_operations() {
    let mut native = NativeSession::new();
    eval_int(&mut native, r#""Z".[0] as Signed64"#, 90);
}

#[test]
fn template_expression_is_rejected_until_library_defined_formatting_is_supported() {
    let mut native = NativeSession::new();
    let generations = native
        .session
        .live_runtime
        .generations()
        .iter()
        .map(|generation| generation.id)
        .collect::<Vec<_>>();
    let submission = native.session.submit(r#""value \(42)""#);
    let Submission::Rejected { diagnostics } = submission else {
        panic!("unexpected submission: {submission:?}");
    };
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.diagnostic().code.slug() == "parse-template-strings-not-supported"
        }),
        "{diagnostics:?}"
    );
    assert_eq!(
        native
            .session
            .live_runtime
            .generations()
            .iter()
            .map(|generation| generation.id)
            .collect::<Vec<_>>(),
        generations
    );
}

#[test]
fn expression_can_use_prior_zero_parameter_function() {
    let mut native = NativeSession::new();
    declared(&mut native, "a() Signed64: return 1 + 1");

    eval_int(&mut native, "3 + a", 5);
}

#[test]
fn package_callable_executes_with_the_typed_evaluator() {
    let mut package = PackageSession::new();
    let declarations = package.session.declarations().to_string();
    assert!(!declarations.contains("signed64_add(lhs"));

    let submission = package.session.submit("40 + 2");
    assert!(
        matches!(
            submission,
            Submission::Evaluated {
                value: crate::value::EvaluatedValue {
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
    declared(&mut native, "value Signed64 := 40");

    eval_int(&mut native, "value + 2", 42);
}

#[test]
fn expression_uses_latest_redefinition_slot() {
    let mut native = NativeSession::new();
    declared(&mut native, "value Signed64 := 40");
    declared(&mut native, "value Signed64 := 41");

    eval_int(&mut native, "value + 1", 42);
}

#[test]
fn declaration_generates_live_runtime_entry() {
    let mut native = NativeSession::new();
    let before = native.session.live_runtime().generations().len();
    declared(&mut native, "value Signed64 := 40");

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
fn active_generation_is_retained_during_active_call_and_retired_after_completion() {
    let mut native = NativeSession::new();
    declared(&mut native, "value Signed64 := 40");
    let before = native.session.live_runtime().generations().len();

    let active_call = {
        let symbols = native.session.live_runtime().symbols().clone();
        native
            .session
            .live_runtime_mut()
            .track_active_generations(symbols.values().copied())
    };

    declared(&mut native, "value Signed64 := 41");
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
    declared(&mut native, "value Signed64 := 40");

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

    declared(&mut native, "value Signed64 := 40");
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
    declared(&mut native, "value Signed64 := 40");
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
    declared(&mut native, "value Signed64 := 40");
    let before = native.session.live_runtime().generations().len();

    declared(&mut native, "value Signed64 := 41");
    assert_eq!(
        native.session.live_runtime().generation_for_symbol("value"),
        Some(GenerationId(before as u64 + 1)),
    );
    assert_eq!(native.session.live_runtime().generations().len(), before);
}

#[test]
fn redeclaration_preserves_siblings_and_their_live_generations() {
    let mut native = NativeSession::new();
    declared(&mut native, "value Signed64 := 40\nother Signed64 := 2");
    let sibling_generation = native.session.live_runtime().generation_for_symbol("other");
    let sibling_slot = native.session.source.symbol_slot("other");

    declared(&mut native, "value Signed64 := 42");
    eval_int(&mut native, "value", 42);
    eval_int(&mut native, "other", 2);
    assert_eq!(
        native.session.live_runtime().generation_for_symbol("other"),
        sibling_generation
    );
    assert_eq!(native.session.source.symbol_slot("other"), sibling_slot);

    let before = native.session.declarations().to_owned();
    assert!(matches!(
        native.session.submit("value Signed64 := missing"),
        Submission::Rejected { .. }
    ));
    assert_eq!(native.session.declarations(), before);
    eval_int(&mut native, "value", 42);
    eval_int(&mut native, "other", 2);

    let saved = native
        .session
        .materialize()
        .expect("valid declarations")
        .to_owned();
    let mut replay = InterpreterSession::default();
    assert!(matches!(
        replay.submit(&saved),
        Submission::DeclarationAccepted { .. }
    ));
    for (expression, expected) in [("value", 42), ("other", 2)] {
        let result = replay.submit(expression);
        assert!(
            matches!(result, Submission::Evaluated { value, .. } if value.storage == RuntimeStorage::Int64(expected))
        );
    }
}

#[test]
fn public_redefinitions_after_a_private_section_reject_without_visibility_drift() {
    let mut session = InterpreterSession::default();
    let source = "visible := 1\nprivate\nsecret := 2";
    assert!(matches!(
        session.submit(source),
        Submission::DeclarationAccepted { .. }
    ));
    for input in ["visible := 3", "another := 4", "PublicTag is Unit"] {
        rejected_with_code(session.submit(input), "interpreter-private-section");
        assert_eq!(session.declarations(), format!("{source}\n"));
    }
    let saved = session
        .materialize()
        .expect("accepted source remains valid");
    let parsed = saved.parse_source_full();
    assert!(
        !parsed
            .ast
            .private_defs
            .contains(&Intern::from_ref("visible"))
    );
    assert!(
        parsed
            .ast
            .private_defs
            .contains(&Intern::from_ref("secret"))
    );
    assert!(
        matches!(session.submit("visible"), Submission::Evaluated { value, .. } if value.storage == RuntimeStorage::Int64(1))
    );
}

#[test]
fn redeclaration_of_incompatible_scalar_type_is_rejected() {
    let mut package = PackageSession::new();
    assert!(matches!(
        package.session.submit("value Small := 1"),
        Submission::DeclarationAccepted { .. }
    ));

    let response = package.session.submit("value Signed64 := 2");
    rejected_with_code(response, "interpreter-incompatible-redefinition");
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
    declared(&mut native, "double(x Signed64) Signed64: return x");
    let before = native.session.live_runtime().generations().len();

    declared(&mut native, "double(x Signed64) Signed64: return x");
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
    declared(&mut native, "double(x Signed64) Signed64: return x");

    let before = native.session.live_runtime().generations().len();
    let response = native
        .session
        .submit("double(x Signed64, y Signed64) Signed64: return x + y");
    rejected_with_code(response, "interpreter-incompatible-redefinition");
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
    declared(&mut native, "double(x Signed64) Signed64: return x");

    let response = native
        .session
        .submit("double(x ref Signed64) Signed64: return 0");
    rejected_with_code(response, "interpreter-incompatible-redefinition");
}

#[test]
fn function_redefinition_rejects_value_kind_mismatch() {
    let mut native = NativeSession::new();
    declared(&mut native, "double(x Signed64) Signed64: return x");
    let response = native.session.submit("double Signed64 := 12");
    rejected_with_code(response, "interpreter-incompatible-redefinition");
}

#[test]
fn declaration_stores_runtime_value_for_scalar_type() {
    let mut native = NativeSession::new();
    declared(&mut native, "value Signed64 := 40");
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
    declared(&mut native, "value Signed64 := 40");
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
#[ignore = "debug"]
fn debug_evaluation_candidate_metadata() {
    let mut native = NativeSession::new();
    declared(&mut native, "small Small := 7");
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

    declared(
        &mut native,
        "forty_two() Signed64:\n    result Signed64 := 42\n    return result",
    );
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
    rejected_with_code(response, "interpreter-invalid-task-id");
}

#[test]
fn run_command_requires_expression_argument() {
    let mut native = NativeSession::new();

    let response = native.session.submit("#run");
    rejected_with_code(response, "interpreter-invalid-command-usage");
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

    declared(&mut native, "value Signed64 := 40");
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
    declared(&mut native, "value Signed64 := 40");

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
fn aggregate_results_retain_values_and_types() {
    let mut native = NativeSession::new();
    declared(&mut native, "small Small := 7");

    let Submission::Evaluated { value, .. } = native.session.submit("[small, small]") else {
        panic!("aggregate result should evaluate");
    };
    assert_eq!(value.display(), "[7, 7]");
    assert!(value.gin_type.is_some());
    assert!(native.session.pending().is_none());
}

#[test]
fn empty_image_has_empty_scope_and_no_pending_condition() {
    let session = InterpreterSession::default();
    assert!(session.scope().is_empty());
    assert!(session.pending().is_none());
    assert!(session.restarts().is_empty());
}

#[test]
fn expression_evaluates_without_any_declared_type() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("40 + 2"),
        Submission::Evaluated {
            value: crate::value::EvaluatedValue {
                storage: RuntimeStorage::Int64(42),
                ..
            },
            ..
        }
    ));
}

#[test]
fn inferred_value_declaration_evaluates_without_a_native_abi() {
    let mut session = InterpreterSession::default();
    assert!(matches!(
        session.submit("count := 40 + 2"),
        Submission::DeclarationAccepted { .. }
    ));
    let Submission::Evaluated { value, diagnostics } = session.submit("count") else {
        panic!("a singleton-typed integer should evaluate");
    };
    assert_eq!(value.storage, RuntimeStorage::Int64(42));
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.display_path() != "<interpreter-generated>"),
        "{diagnostics:?}"
    );
}

#[test]
fn scope_lists_bootstrapped_declarations() {
    let native = NativeSession::new();
    let scope = native.session.scope();
    assert!(
        scope.iter().any(|entry| entry.name.contains("Signed64")),
        "{scope:?}"
    );
}

#[test]
fn type_query_reports_expression_type() {
    let mut native = NativeSession::new();
    declared(&mut native, "value Signed64 := 40");
    match native.session.type_of("value + 2") {
        QueryOutcome::Answered(text) => assert!(text.contains("Signed64"), "{text}"),
        other => panic!("expected type answer, got {other:?}"),
    }
}

#[test]
fn type_query_rejects_unknown_symbol() {
    let mut native = NativeSession::new();
    match native.session.type_of("missing + 2") {
        QueryOutcome::Rejected(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.diagnostic().code.slug() == "type-unknown-symbol"),
            "expected unknown symbol diagnostic",
        ),
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn failed_submission_offers_retry_and_abort() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("missing + 2"),
        Submission::Rejected { .. }
    ));
    assert_eq!(native.session.pending(), Some("missing + 2"));
    assert!(native.session.restarts().contains(&Restart::Retry));
    assert!(native.session.restarts().contains(&Restart::Abort));
    assert!(native.session.abort());
    assert!(native.session.pending().is_none());
    assert!(native.session.restarts().is_empty());
}

#[test]
fn retry_after_defining_the_symbol_evaluates() {
    let mut native = NativeSession::new();
    assert!(matches!(
        native.session.submit("missing + 2"),
        Submission::Rejected { .. }
    ));
    declared(&mut native, "missing Signed64 := 40");
    match native.session.retry() {
        Some(Submission::Evaluated {
            value:
                crate::value::EvaluatedValue {
                    storage: RuntimeStorage::Int64(42),
                    ..
                },
            ..
        }) => {}
        other => panic!("expected 42 after retry, got {other:?}"),
    }
    assert!(native.session.pending().is_none());
}

#[test]
fn materialize_emits_only_accepted_source() {
    let mut native = NativeSession::new();
    native.session.submit("missing + 2");
    let source = native.session.materialize().unwrap().to_string();
    assert!(source.contains("signed64_add_bits"), "{source}");
    assert!(!source.contains("missing + 2"), "{source}");
}

#[test]
fn materialized_image_recompiles_clean() {
    let mut native = NativeSession::new();
    let source = native.session.materialize().unwrap().to_string();
    let mut replay = InterpreterSession::default();
    assert!(
        matches!(
            replay.submit(&source),
            Submission::DeclarationAccepted { .. }
        ),
        "materialized image should be accepted",
    );
}
