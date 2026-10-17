use super::*;
use diagnostic::code::InterpreterCode;

impl InterpreterSession {
    pub fn inspect(&mut self, expression: &str) -> Submission {
        if expression.trim().is_empty() {
            return Submission::Answered {
                text: "usage: #inspect <expression>".to_owned(),
            };
        }
        let snapshot = match self.eval_state.lock() {
            Ok(state) => state.clone(),
            Err(_) => {
                return Submission::Rejected {
                    diagnostics: vec![submitted_diagnostic(
                        expression,
                        // interpreter-evaluator-state-unavailable
                        Diagnostic::new(
                            InterpreterCode::EvaluatorStateUnavailable,
                            "evaluator state is unavailable after a worker panic",
                        ),
                    )],
                };
            }
        };
        let evaluation = self.source.evaluation_candidate_with_dispatches(
            expression,
            self.next_generation,
            &HashMap::new(),
        );
        let checked = self.check(evaluation.source());
        let submission = evaluate_checked_submission(
            &checked,
            &evaluation,
            Arc::new(AtomicBool::new(false)),
            &Mutex::new(snapshot),
        );
        match submission {
            Submission::Evaluated { value, diagnostics } => {
                Submission::Inspected { value, diagnostics }
            }
            submission => submission,
        }
    }
}

#[cfg(test)]
#[path = "tests/inspection_tests.rs"]
mod tests;
