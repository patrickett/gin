use crate::value::EvaluatedValue;
use diagnostic::code::{EvaluationCode, InterpreterCode};
use diagnostic::{Diagnostic, DiagnosticCode};
use ginc::driver::CheckedPackage;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use typecheck::{EvalHost, EvalState, Evaluator, Fuel};

pub(crate) enum Execution {
    Completed(EvaluatedValue),
    Failed(ExecutionFailure),
    Cancelled(ExecutionFailure),
}

pub(crate) type ExecutionFailure = typecheck::EvalFailure;

pub(crate) fn execute(
    checked: &CheckedPackage,
    symbol: &str,
    stop_requested: Arc<AtomicBool>,
    state: &Mutex<EvalState>,
) -> Execution {
    let host = InteractiveHost {
        target: checked.target(),
        stop_requested: Arc::clone(&stop_requested),
    };
    if host.cancelled() {
        return cancelled();
    }
    let file = checked.primary_file().typed();
    let Some(entry) = file
        .defs
        .keys()
        .find(|name| name.0.as_str() == symbol)
        .copied()
    else {
        return Execution::Failed(
            // interpreter-missing-expression
            Diagnostic::new(
                InterpreterCode::MissingExpression,
                "submission has no expression to evaluate",
            )
            .into(),
        );
    };
    let evaluator = Evaluator::new(file.resolved_program(), host).with_programs(
        checked
            .files()
            .iter()
            .map(|file| file.typed().resolved_program()),
    );
    let mut outcome = execute_call(evaluator, entry, &stop_requested, state);
    if let Execution::Completed(value) = &mut outcome {
        value.gin_type = Some(file.defs[&entry].return_type.clone());
    }
    outcome
}

fn execute_call(
    mut evaluator: Evaluator<'_, impl EvalHost>,
    entry: typecheck::DefId,
    stop_requested: &AtomicBool,
    state: &Mutex<EvalState>,
) -> Execution {
    let mut state = match state.lock() {
        Ok(state) => state,
        Err(_) => {
            return Execution::Failed(
                // interpreter-evaluator-state-unavailable
                Diagnostic::new(
                    InterpreterCode::EvaluatorStateUnavailable,
                    "evaluator state is unavailable after a worker panic",
                )
                .into(),
            );
        }
    };
    let mut candidate = state.clone();
    candidate.fuel = Fuel::default();
    let outcome = match evaluator.call_with_state(entry, &[], &mut candidate) {
        Ok(value) => Execution::Completed(EvaluatedValue::new(value, None)),
        Err(diagnostic) => {
            let was_cancelled =
                diagnostic.code == DiagnosticCode::Evaluation(EvaluationCode::Cancelled);
            let failure = evaluator.take_failure(diagnostic);
            if was_cancelled {
                Execution::Cancelled(failure)
            } else {
                Execution::Failed(failure)
            }
        }
    };
    if matches!(outcome, Execution::Completed(_)) {
        if stop_requested.load(Ordering::SeqCst) {
            return cancelled();
        }
        *state = candidate;
    }
    outcome
}

fn cancelled() -> Execution {
    Execution::Cancelled(
        // compile-time-cancelled
        Diagnostic::new(EvaluationCode::Cancelled, "evaluation was cancelled").into(),
    )
}

struct InteractiveHost<'a> {
    target: &'a flask::CompileTarget,
    stop_requested: Arc<AtomicBool>,
}

impl EvalHost for InteractiveHost<'_> {
    fn target(&self) -> &flask::CompileTarget {
        self.target
    }

    fn cancelled(&self) -> bool {
        self.stop_requested.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
use crate::RuntimeStorage;
#[cfg(test)]
use ast::ConstValue;

#[cfg(test)]
#[path = "tests/transaction_tests.rs"]
mod transaction_tests;
