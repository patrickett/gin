use ginterpreter::{InterpreterSession, Submission};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::io;

pub(super) enum Input {
    Terminal(DefaultEditor),
    Stdin,
}

impl Input {
    pub(super) fn new(interactive: bool) -> Result<Self, ReadlineError> {
        if interactive {
            DefaultEditor::new().map(Self::Terminal)
        } else {
            Ok(Self::Stdin)
        }
    }

    fn readline(&mut self, prompt: &str) -> Result<String, ReadlineError> {
        match self {
            Self::Terminal(editor) => {
                let line = editor.readline(prompt)?;
                if !line.trim().is_empty() {
                    editor.add_history_entry(&line)?;
                }
                Ok(line)
            }
            Self::Stdin => {
                let mut line = String::new();
                if io::stdin().read_line(&mut line)? == 0 {
                    return Err(ReadlineError::Eof);
                }
                Ok(line.trim_end_matches(['\r', '\n']).to_string())
            }
        }
    }

    pub(super) fn read_submission(
        &mut self,
        session: &mut InterpreterSession,
    ) -> Result<Option<Submission>, ReadlineError> {
        let input = match self.readline("> ") {
            Ok(input) => input,
            Err(ReadlineError::Eof) => return Ok(None),
            Err(error) => return Err(error),
        };
        let submission = session.submit(&input);
        if !matches!(submission, Submission::Incomplete) {
            return Ok(Some(submission));
        }

        let mut buffer = format!("{input}\n");
        loop {
            match self.readline("| ") {
                Ok(line) if line.trim().is_empty() => {
                    let submission = session.submit(buffer.trim_end());
                    if matches!(submission, Submission::Incomplete) {
                        eprintln!("submission is still incomplete");
                        continue;
                    }
                    return Ok(Some(submission));
                }
                Ok(line) => {
                    buffer.push_str(&line);
                    buffer.push('\n');
                }
                Err(ReadlineError::Eof) => {
                    let submission = session.submit(buffer.trim_end());
                    if matches!(submission, Submission::Incomplete) {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "input ended during an incomplete submission",
                        )
                        .into());
                    }
                    return Ok(Some(submission));
                }
                Err(error) => return Err(error),
            }
        }
    }
}
