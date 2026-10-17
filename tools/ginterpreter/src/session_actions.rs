use super::{InterpreterSession, Submission, SubmissionKind, declaration_names};
use crate::CodeAction;
use diagnostic::Span;
use ginfmt::help::HelpContext;
use parser::query::SourceParseExt;

impl InterpreterSession {
    pub fn code_actions(&mut self, input: &str) -> Vec<CodeAction> {
        let input = input.trim_end();
        if input.trim().is_empty() {
            return Vec::new();
        }
        let parsed = input.parse_source_full();
        let source = match SubmissionKind::from_ast(&parsed.ast) {
            SubmissionKind::Declaration => self
                .source
                .candidate_without_shadowed(input, &declaration_names(&parsed.ast)),
            SubmissionKind::Expression | SubmissionKind::Mixed => self.source.candidate(input),
        };
        let offset = source.len() - input.len() - 1;
        let parsed = source.parse_source_full();
        let checked = self.check(&source);
        let file = checked.primary_file();
        let package: Vec<_> = checked.files().iter().map(|file| file.typed()).collect();
        HelpContext {
            source: file.source(),
            parsed: &parsed,
            typed: file.typed(),
            package: &package,
        }
        .suggestions()
        .into_iter()
        .filter_map(|help| submission_action(help, offset, input.len()))
        .collect()
    }

    pub(super) fn suggestions_report(&mut self, input: Option<&str>) -> Submission {
        let input = input
            .map(str::trim)
            .filter(|input| !input.is_empty())
            .unwrap_or_else(|| self.pending().unwrap_or(self.declarations()))
            .to_owned();
        let actions = self.code_actions(&input);
        let text = if actions.is_empty() {
            "no suggested edits".to_owned()
        } else {
            actions
                .iter()
                .map(|action| {
                    let edits = action
                        .edits
                        .iter()
                        .map(|edit| {
                            format!(
                                "  bytes {}..{}: {:?}",
                                edit.span.start(),
                                edit.span.end(),
                                edit.replacement
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!(
                        "{}: {}\n{edits}",
                        action.diagnostic.code.slug(),
                        action.diagnostic.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Submission::Answered { text }
    }
}

fn submission_action(mut action: CodeAction, offset: usize, length: usize) -> Option<CodeAction> {
    let local_span = |span: Span| {
        let start = span.start().checked_sub(offset)?;
        let end = span.end().checked_sub(offset)?;
        (end <= length).then(|| Span::new(start, end))
    };
    action.diagnostic.span = local_span(action.diagnostic.span)?;
    for edit in &mut action.edits {
        edit.span = local_span(edit.span)?;
    }
    Some(action)
}
