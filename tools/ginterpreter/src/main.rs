use clap::Parser;
use ginterpreter::{
    Command, InterpreterContext, InterpreterSession, RuntimeStorage, RuntimeValue, Submission,
    TaskResult, TaskState,
};
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
    println!("Gin Interpreter");
    println!("Expressions are compiled and executed natively.");
    println!("Type #help for commands.");

    let mut editor = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(error) => {
            eprintln!("error: failed to initialize terminal input: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args = Args::parse();
    let mut session = match args.package.as_deref() {
        Some(path) => match InterpreterContext::load(path) {
            Ok(context) => InterpreterSession::with_context(context),
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        },
        None => match InterpreterSession::standalone() {
            Ok(session) => session,
            Err(error) => {
                eprintln!("error: {error}");
                return ExitCode::FAILURE;
            }
        },
    };
    if args.package.is_some() {
        println!("Package: {}", session.context().package().root().display());
    }

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

        process_task_results(&mut session);

        match submission {
            Submission::DeclarationAccepted { diagnostics } => print_diagnostics(&diagnostics),
            Submission::Evaluated {
                value:
                    RuntimeValue {
                        storage: RuntimeStorage::Int64(value),
                        ..
                    },
                diagnostics,
            } => {
                print_diagnostics(&diagnostics);
                println!("{value}");
            }
            Submission::Evaluated {
                value:
                    RuntimeValue {
                        storage: RuntimeStorage::Bytes(_),
                        ..
                    },
                diagnostics,
                ..
            } => {
                print_diagnostics(&diagnostics);
                eprintln!("runtime value cannot be printed");
            }
            Submission::Command(Command::Clear) => println!("session cleared"),
            Submission::Command(Command::Help) => print_help(),
            Submission::Command(Command::Reset) => println!("session and running tasks reset"),
            Submission::Command(Command::Tasks) => print_task_summary(&session),
            Submission::Command(Command::TaskStarted(task_id)) => {
                println!("started task {id}", id = task_id.0);
            }
            Submission::Command(Command::TaskStopRequested(task_id)) => {
                println!("requested stop for task {}", task_id.0);
            }
            Submission::Command(Command::Quit) => break,
            Submission::Empty | Submission::Incomplete => {}
            Submission::Rejected { diagnostics } => print_diagnostics(&diagnostics),
            Submission::UnknownCommand(command) => {
                eprintln!("unknown command `#{command}`; type #help for commands")
            }
        }

        process_task_results(&mut session);
    }

    ExitCode::SUCCESS
}

fn print_help() {
    println!("#clear  discard all accepted submissions");
    println!("#reset  clear declarations, runtime values, and running tasks");
    println!("#tasks  list running tasks");
    println!("#run    run an expression asynchronously");
    println!("#stop   request cancellation of a running task");
    println!("#help   show this help");
    println!("#exit   exit the REPL");
    println!("#quit   exit the REPL");
    println!("blank   finish a multiline submission");
    println!("Ctrl-C  cancel the current input");
    println!("Ctrl-D  exit the REPL");
}

fn print_diagnostics(diagnostics: &[ginterpreter::InterpreterDiagnostic]) {
    for diagnostic in diagnostics {
        diagnostic.print();
    }
}

fn print_task_summary(session: &InterpreterSession) {
    let tasks = session.task_summaries();
    if tasks.is_empty() {
        println!("no running tasks");
        return;
    }

    for task in tasks {
        println!("task {} running: {}", task.id.0, task.expression);
    }
}

fn process_task_results(session: &mut InterpreterSession) {
    for task in session.poll_task_results() {
        println!("task {} completed: {}", task.id.0, task.expression);
        handle_task_result(task);
    }
}

fn handle_task_result(task: TaskResult) {
    match (task.state, task.submission) {
        (TaskState::Cancelled, submission) | (TaskState::Failed, submission) => {
            if let ginterpreter::Submission::Rejected { diagnostics } = submission {
                print_diagnostics(&diagnostics);
            } else {
                eprintln!("task {} ended unexpectedly", task.id.0);
            }
        }
        (TaskState::Completed, submission) => match submission {
            ginterpreter::Submission::Evaluated {
                value:
                    RuntimeValue {
                        storage: RuntimeStorage::Int64(value),
                        ..
                    },
                diagnostics,
            } => {
                print_diagnostics(&diagnostics);
                println!("task {} => {value}", task.id.0);
            }
            ginterpreter::Submission::Evaluated {
                value:
                    RuntimeValue {
                        storage: RuntimeStorage::Bytes(_),
                        ..
                    },
                diagnostics,
                ..
            } => {
                print_diagnostics(&diagnostics);
                eprintln!("task {} result cannot be printed", task.id.0);
            }
            ginterpreter::Submission::Rejected { diagnostics } => {
                print_diagnostics(&diagnostics);
            }
            _ => {}
        },
        (TaskState::Running, _) => {}
    }
}
