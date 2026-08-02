use crate::ReplContext;
use diagnostic::{Category, Diagnostic};
use parser::query::SourceParseExt;
use resolve::ParsedFile;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Clear,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionKind {
    Declaration,
    Expression,
    Mixed,
}

#[derive(Debug)]
pub enum Submission {
    Accepted {
        kind: SubmissionKind,
        source: String,
        diagnostics: Vec<Diagnostic>,
    },
    Command(Command),
    Empty,
    Incomplete,
    Rejected {
        source: String,
        diagnostics: Vec<Diagnostic>,
    },
    UnknownCommand(String),
}

pub struct ReplSession {
    context: ReplContext,
    source: String,
}

impl Default for ReplSession {
    fn default() -> Self {
        Self::with_context(ReplContext::isolated())
    }
}

impl ReplSession {
    pub fn with_context(context: ReplContext) -> Self {
        Self {
            context,
            source: String::new(),
        }
    }

    pub fn context(&self) -> &ReplContext {
        &self.context
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn submit(&mut self, input: &str) -> Submission {
        let input = input.trim_end();
        if input.trim().is_empty() {
            return Submission::Empty;
        }

        if let Some(command) = input.trim().strip_prefix(':') {
            return self.command(command);
        }

        let candidate = match self.source.is_empty() {
            true => format!("{input}\n"),
            false => format!("{}{input}\n", self.source),
        };
        let submission_ast = input.parse_source_full().ast;
        let kind = SubmissionKind::from_ast(&submission_ast);
        let parsed = candidate.parse_source_full();
        if parsed.is_incomplete() {
            return Submission::Incomplete;
        }
        if has_flaws(&parsed.symptoms) {
            return Submission::Rejected {
                source: candidate,
                diagnostics: parsed.symptoms,
            };
        }

        let session_path = self.context.package().session_path().to_path_buf();
        let mut files = self.context.package().files().to_vec();
        files.push(ParsedFile {
            path: session_path.clone(),
            source: candidate.clone(),
            output: parsed,
        });
        let mut files = resolve::resolve_imports(files, self.context.package().dependencies());
        let mut asts: Vec<_> = files.iter().map(|file| file.output.ast.clone()).collect();
        let prepare_diagnostics = typecheck::prepare_package_asts(&mut asts, self.context.target());
        for (file, (ast, diagnostics)) in files
            .iter_mut()
            .zip(asts.into_iter().zip(prepare_diagnostics))
        {
            file.output.ast = ast;
            file.output.symptoms.extend(diagnostics);
        }

        let transformed = transform_package_with_shared_context(
            files.iter().map(|file| file.output.ast.clone()).collect(),
            PackageTransformOptions::FULL,
        );
        let session_index = files
            .iter()
            .position(|file| file.path == session_path)
            .expect("resolved package retains the REPL session file");
        let session_file = &mut files[session_index];
        session_file.output.symptoms.extend(
            transformed.typed_asts[session_index]
                .all_flaws()
                .into_iter()
                .map(|(_, diagnostic)| diagnostic.clone()),
        );
        if has_flaws(&session_file.output.symptoms) {
            return Submission::Rejected {
                source: candidate,
                diagnostics: std::mem::take(&mut session_file.output.symptoms),
            };
        }

        self.source = candidate;
        Submission::Accepted {
            kind,
            source: self.source.clone(),
            diagnostics: std::mem::take(&mut session_file.output.symptoms),
        }
    }

    fn command(&mut self, command: &str) -> Submission {
        match command.trim() {
            "clear" => {
                self.source.clear();
                Submission::Command(Command::Clear)
            }
            "help" => Submission::Command(Command::Help),
            "exit" | "quit" | "q" => Submission::Command(Command::Quit),
            command => Submission::UnknownCommand(command.to_string()),
        }
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

fn has_flaws(diagnostics: &[Diagnostic]) -> bool {
    diagnostics
        .iter()
        .any(|diagnostic| diagnostic.category == Category::Flaw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_submissions_accumulate_source() {
        let mut session = ReplSession::default();

        assert!(matches!(
            session.submit("Int is in 0...100"),
            Submission::Accepted { .. }
        ));
        assert!(matches!(
            session.submit("answer := 42"),
            Submission::Accepted { .. }
        ));
        assert!(matches!(
            session.submit("identity(value Int) Int := value"),
            Submission::Accepted { .. }
        ));
        assert_eq!(
            session.source(),
            "Int is in 0...100\nanswer := 42\nidentity(value Int) Int := value\n"
        );
    }

    #[test]
    fn function_header_requests_more_input() {
        let mut session = ReplSession::default();

        assert!(matches!(
            session.submit("double(value Int) Int:"),
            Submission::Incomplete
        ));
        assert!(session.source().is_empty());
    }

    #[test]
    fn multiline_function_is_accepted_atomically() {
        let mut session = ReplSession::default();
        let _ = session.submit("Int is in 0...100");

        assert!(matches!(
            session.submit("forty_two() Int:\n    result := 42\nreturn result"),
            Submission::Accepted { .. }
        ));
        assert_eq!(
            session.source(),
            "Int is in 0...100\nforty_two() Int:\n    result := 42\nreturn result\n"
        );
    }

    #[test]
    fn submissions_are_classified_from_their_ast() {
        let mut session = ReplSession::default();

        assert!(matches!(
            session.submit("answer := 42"),
            Submission::Accepted {
                kind: SubmissionKind::Declaration,
                ..
            }
        ));
        assert!(matches!(
            session.submit("answer + 1"),
            Submission::Accepted {
                kind: SubmissionKind::Expression,
                ..
            }
        ));
    }

    #[test]
    fn type_flaws_reject_submission_without_changing_source() {
        let mut session = ReplSession::default();
        let _ = session.submit("answer := 42");

        assert!(matches!(
            session.submit("missing(answer)"),
            Submission::Rejected { .. }
        ));
        assert_eq!(session.source(), "answer := 42\n");
    }

    #[test]
    fn rejected_submission_does_not_change_source() {
        let mut session = ReplSession::default();
        let _ = session.submit("answer := 42");

        assert!(matches!(
            session.submit("broken := @"),
            Submission::Rejected { .. }
        ));
        assert_eq!(session.source(), "answer := 42\n");
    }

    #[test]
    fn clear_discards_accumulated_source() {
        let mut session = ReplSession::default();
        let _ = session.submit("answer := 42");

        assert!(matches!(
            session.submit(":clear"),
            Submission::Command(Command::Clear)
        ));
        assert!(session.source().is_empty());
    }

    #[test]
    fn exit_is_a_quit_alias() {
        let mut session = ReplSession::default();

        assert!(matches!(
            session.submit(":exit"),
            Submission::Command(Command::Quit)
        ));
    }

    #[test]
    fn unknown_commands_are_reported() {
        let mut session = ReplSession::default();

        assert!(matches!(
            session.submit(":wat"),
            Submission::UnknownCommand(command) if command == "wat"
        ));
    }
}
