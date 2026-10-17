use diagnostic::{Diagnostic, Span};

#[derive(Debug, Clone)]
pub struct InterpreterDiagnostic {
    source: String,
    display_path: String,
    diagnostic: Diagnostic,
    frames: Vec<crate::InterpreterFrame>,
}

impl InterpreterDiagnostic {
    pub fn new(
        source: impl Into<String>,
        display_path: impl Into<String>,
        diagnostic: Diagnostic,
    ) -> Self {
        Self {
            source: source.into(),
            display_path: display_path.into(),
            diagnostic,
            frames: Vec::new(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn display_path(&self) -> &str {
        &self.display_path
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn frames(&self) -> &[crate::InterpreterFrame] {
        &self.frames
    }

    pub(crate) fn with_frames(mut self, frames: Vec<crate::InterpreterFrame>) -> Self {
        self.frames = frames;
        self
    }

    pub(crate) fn rebase_submission(&mut self, candidate: &str, source: &str, start: usize) {
        assert_eq!(candidate.get(start..start + source.len()), Some(source));
        let range = start..start + source.len();
        if self.display_path == "<interpreter>" && self.source == candidate {
            let span = relative_span(self.diagnostic.span, &range);
            let related_fit = self
                .diagnostic
                .related()
                .iter()
                .all(|related| relative_span(related.span, &range).is_some());
            if let Some(span) = span.filter(|_| related_fit) {
                self.diagnostic.span = span;
                for related in self.diagnostic.related_mut() {
                    related.span = relative_span(related.span, &range)
                        .expect("related spans were checked before rebasing");
                }
                self.source = source.to_owned();
            }
        }
        for frame in &mut self.frames {
            if frame.display_path == "<interpreter>" && frame.source.as_ref() == candidate {
                if let Some(span) = relative_span(frame.span, &range) {
                    frame.span = span;
                    frame.source = std::sync::Arc::from(source);
                }
            }
        }
    }
}

fn relative_span(span: Span, range: &std::ops::Range<usize>) -> Option<Span> {
    if span.start() >= range.start && span.end() <= range.end {
        Some(Span::new(
            span.start() - range.start,
            span.end() - range.start,
        ))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "tests/declaration_diagnostics_tests.rs"]
mod tests;
