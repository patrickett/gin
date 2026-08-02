use clap::Parser;
use diagnostic::Diagnostic;
use ginrepl::{Command, ReplContext, ReplSession, Submission};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    package: Option<PathBuf>,
}

fn main() -> ExitCode {
    println!("Gin REPL (frontend preview)");
    println!("Submissions are checked and retained; execution is not available yet.");
    println!("Type :help for commands.");

    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(error) => {
            eprintln!("error: failed to initialize terminal input: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args = Args::parse();
    let context = match args.package.as_deref() {
        Some(path) => match ReplContext::load(path) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => ReplContext::isolated(),
    };
    if args.package.is_some() {
        println!("Package: {}", context.package().root().display());
    }
    let mut session = ReplSession::with_context(context);

    'repl: loop {
        let input = match editor.readline("> ") {
            Ok(input) => input,
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(error) => {
                eprintln!("error: failed to read input: {error}");
                return ExitCode::FAILURE;
            }
        };

        if !input.trim().is_empty() {
            let _ = editor.add_history_entry(&input);
        }

        let mut submission = session.submit(&input);
        if matches!(submission, Submission::Incomplete) {
            let mut buffer = format!("{input}\n");
            loop {
                match editor.readline("| ") {
                    Ok(line) if line.trim().is_empty() => {
                        submission = session.submit(buffer.trim_end());
                        if matches!(submission, Submission::Incomplete) {
                            println!("submission is still incomplete");
                            continue;
                        }
                        break;
                    }
                    Ok(line) => {
                        let _ = editor.add_history_entry(&line);
                        buffer.push_str(&line);
                        buffer.push('\n');
                    }
                    Err(ReadlineError::Interrupted) => {
                        println!("^C");
                        continue 'repl;
                    }
                    Err(ReadlineError::Eof) => break 'repl,
                    Err(error) => {
                        eprintln!("error: failed to read input: {error}");
                        return ExitCode::FAILURE;
                    }
                }
            }
        }

        match submission {
            Submission::Accepted {
                source,
                diagnostics,
                ..
            } => print_diagnostics(&source, &diagnostics),
            Submission::Command(Command::Clear) => println!("session cleared"),
            Submission::Command(Command::Help) => print_help(),
            Submission::Command(Command::Quit) => break,
            Submission::Empty | Submission::Incomplete => {}
            Submission::Rejected {
                source,
                diagnostics,
            } => print_diagnostics(&source, &diagnostics),
            Submission::UnknownCommand(command) => {
                eprintln!("unknown command `:{command}`; type :help for commands")
            }
        }
    }

    ExitCode::SUCCESS
}

fn print_help() {
    println!(":clear  discard all accepted submissions");
    println!(":help   show this help");
    println!(":exit   exit the REPL");
    println!(":quit   exit the REPL");
    println!("blank   finish a multiline submission");
    println!("Ctrl-C  cancel the current input");
    println!("Ctrl-D  exit the REPL");
}

fn print_diagnostics(source: &str, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        diagnostic.print(source, "<repl>");
    }
}
