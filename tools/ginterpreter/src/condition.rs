use crate::{InterpreterDiagnostic, Restart};
use diagnostic::Span;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct InterpreterFrame {
    pub function: String,
    pub source: Arc<str>,
    pub display_path: String,
    pub span: Span,
    pub locals: Vec<typecheck::EvalLocal>,
}

#[derive(Debug, Clone)]
pub struct InterpreterCondition {
    pub submission: String,
    pub diagnostics: Vec<InterpreterDiagnostic>,
}

impl InterpreterCondition {
    pub fn frames(&self) -> &[InterpreterFrame] {
        self.diagnostics
            .iter()
            .find(|diagnostic| !diagnostic.frames().is_empty())
            .map(InterpreterDiagnostic::frames)
            .unwrap_or_default()
    }

    pub fn frame(&self, index: usize) -> Option<&InterpreterFrame> {
        self.frames().get(index)
    }

    pub fn restarts(&self) -> &'static [Restart] {
        &[Restart::Retry, Restart::Abort]
    }
}

impl InterpreterFrame {
    pub fn local(&self, name: &str) -> Option<&typecheck::EvalLocal> {
        self.locals.iter().find(|local| local.name.as_str() == name)
    }
}

impl crate::Submission {
    pub(crate) fn condition(&self, submission: &str) -> Option<InterpreterCondition> {
        match self {
            Self::Rejected { diagnostics } | Self::Cancelled { diagnostics } => {
                Some(InterpreterCondition {
                    submission: submission.to_string(),
                    diagnostics: diagnostics.clone(),
                })
            }
            _ => None,
        }
    }
}

impl crate::TaskResult {
    pub fn condition(&self) -> Option<InterpreterCondition> {
        self.submission.condition(&self.expression)
    }
}

pub(crate) fn resolve_frames(
    checked: &ginc::driver::CheckedPackage,
    evaluation: Option<&crate::GeneratedEvaluation>,
    frames: Vec<typecheck::EvalFrame>,
) -> Vec<InterpreterFrame> {
    let mut sources = HashMap::new();
    let expression = evaluation.map(|evaluation| Arc::<str>::from(evaluation.expression()));
    frames
        .into_iter()
        .map(|frame| {
            let file = checked
                .files()
                .iter()
                .find(|file| file.typed().file_id == frame.file)
                .expect("evaluation frames retain a checked source file");
            let typed = file.typed();
            let bind = frame
                .function
                .and_then(|function| typed.defs.get(&function));
            let expression_frame = frame.function.is_some_and(|function| {
                evaluation.is_some_and(|evaluation| function.0.as_str() == evaluation.symbol())
            });
            let function = if expression_frame {
                "<expression>".to_string()
            } else if frame.function.is_none() {
                "<compile-time expression>".to_string()
            } else {
                bind.and_then(|bind| {
                    file.source()
                        .get(typed.span_table.get(bind.name_span).to_range())
                })
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| frame.function.expect("named call frame").0.to_string())
            };
            let span = frame
                .expression
                .and_then(|expression| typed.exprs.span_of(expression))
                .or_else(|| bind.map(|bind| bind.name_span))
                .map(|span| typed.span_table.get(span))
                .unwrap_or(Span::new(0, 0));
            let source = Arc::clone(
                sources
                    .entry(frame.file)
                    .or_insert_with(|| Arc::<str>::from(file.source())),
            );
            let (source, display_path, span) = frame_source(
                file,
                checked.primary_path(),
                evaluation,
                expression_frame,
                span,
                source,
                expression.as_ref(),
            );
            InterpreterFrame {
                function,
                source,
                display_path,
                span,
                locals: frame.locals,
            }
        })
        .collect()
}

fn frame_source(
    file: &ginc::driver::CheckedFile,
    session_path: &std::path::Path,
    evaluation: Option<&crate::GeneratedEvaluation>,
    expression_frame: bool,
    span: Span,
    source: Arc<str>,
    expression: Option<&Arc<str>>,
) -> (Arc<str>, String, Span) {
    use diagnostic::DiagnosticPathExt;
    if file.path() != session_path {
        return (source, file.path().diagnostic_report_path_from_cwd(), span);
    }
    let Some(evaluation) = evaluation else {
        return (source, "<interpreter>".to_string(), span);
    };
    let range = evaluation.expression_range();
    if expression_frame || (span.start() >= range.start && span.end() <= range.end) {
        let span = if expression_frame {
            Span::new(0, evaluation.expression().len())
        } else {
            Span::new(span.start() - range.start, span.end() - range.start)
        };
        (
            Arc::clone(expression.expect("generated expression source")),
            "<interpreter>".to_string(),
            span,
        )
    } else {
        (source, "<interpreter>".to_string(), span)
    }
}
