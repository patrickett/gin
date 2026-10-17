use clap::Parser;
use ginterpreter::{
    Command, EvaluatedValue, InspectionLimits, InterpreterContext, InterpreterSession, Submission,
    TaskId, TaskResult, TaskState,
};
use rustyline::error::ReadlineError;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod input;
mod jsonl;

#[derive(Parser)]
#[command(
    version,
    about = "Try Gin code and query its types with the compiler's typed evaluator",
    after_help = "Start without arguments for a session in the current directory.\n\
        A nearby mod.flask supplies package sources and dependencies;\n\
        otherwise start a scratch session without imports.\n\
        Use --isolated to disable package and import loading explicitly.\n\n\
        Examples:\n  \
        ginterpreter --eval '40 + 2'\n  \
        ginterpreter --eval 'answer := 42' --eval '#type answer' --eval 'answer'\n  \
        ginterpreter --eval '#help'\n  \
        ginterpreter --eval '#queries'\n  \
        ginterpreter --jsonl --isolated\n\n\
        Each --eval is one complete submission, including multiline code.\n\
        Declarations and expressions go in separate submissions.\n\
        Submissions share one session; any failure gives a nonzero exit status.\n\
        With --jsonl, read one JSON request per line; send an id and op=describe to start.\n\
        Otherwise, without --eval, read submissions from the terminal or standard input.\n\
        Piped sessions also return a nonzero status if any submission fails.\n\
        Finish a multiline submission with a blank line or end of input."
)]
struct Args {
    #[arg(help = "Directory or file to locate a package from (defaults to current directory)")]
    package: Option<PathBuf>,
    #[arg(
        long,
        help = "Start a fresh session with package and import loading disabled"
    )]
    isolated: bool,
    #[arg(
        long,
        value_name = "SOURCE",
        allow_hyphen_values = true,
        help = "Submit code or a query and exit; repeat to share a session"
    )]
    eval: Vec<String>,
    #[arg(
        long,
        conflicts_with = "eval",
        help = "Stream JSON requests and responses on stdio; use #help jsonl for examples"
    )]
    jsonl: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let path = args.package.as_deref().unwrap_or(Path::new("."));
    let context = if args.isolated {
        InterpreterContext::isolated_at(path)
    } else {
        InterpreterContext::discover(path)
    };
    let mut session = match context {
        Ok(context) => InterpreterSession::with_context(context),
        Err(error) => {
            if args.jsonl {
                return jsonl::startup_error(&error.to_string());
            }
            eprintln!("error: {error}");
            return ExitCode::FAILURE;
        }
    };
    if args.jsonl {
        return jsonl::run(&mut session);
    }
    if !args.eval.is_empty() {
        return run_submissions(&mut session, &args.eval);
    }
    run_repl(&mut session)
}

fn run_repl(session: &mut InterpreterSession) -> ExitCode {
    let interactive = std::io::stdin().is_terminal();
    let mut input = match input::Input::new(interactive) {
        Ok(input) => input,
        Err(error) => {
            eprintln!("error: failed to initialize input: {error}");
            return ExitCode::FAILURE;
        }
    };
    if interactive {
        print_banner(session);
    }

    let mut failed = false;
    loop {
        let submission = match input.read_submission(session) {
            Ok(Some(submission)) => submission,
            Ok(None) => break,
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(error) => {
                eprintln!("error: failed to read input: {error}");
                return ExitCode::FAILURE;
            }
        };
        failed |= process_task_results(session);
        match print_submission(session, submission) {
            Outcome::Continue => {}
            Outcome::Failed => failed = true,
            Outcome::Quit => {
                session.clear_tasks();
                break;
            }
        }
        failed |= process_task_results(session);
    }

    if interactive {
        ExitCode::SUCCESS
    } else {
        failed |= finish_tasks(session);
        exit_status(failed)
    }
}

fn print_banner(session: &InterpreterSession) {
    println!("Gin Interpreter");
    println!("Known inputs evaluate at compile time through the compiler's typed evaluator.");
    println!("Type #help for commands and examples, #queries for language queries.");
    if session.context().is_isolated() {
        println!("Standalone session (no package).");
    } else {
        println!("Package: {}", session.context().package().root().display());
    }
}

fn run_submissions(session: &mut InterpreterSession, sources: &[String]) -> ExitCode {
    let mut failed = false;
    for source in sources {
        let submission = session.submit(source);
        match print_submission(session, submission) {
            Outcome::Continue => {}
            Outcome::Failed => failed = true,
            Outcome::Quit => {
                session.clear_tasks();
                break;
            }
        }
        failed |= process_task_results(session);
    }
    failed |= finish_tasks(session);
    exit_status(failed)
}

fn finish_tasks(session: &mut InterpreterSession) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut failed = false;
    while !session.task_summaries().is_empty() {
        failed |= process_task_results(session);
        if std::time::Instant::now() >= deadline {
            session.clear_tasks();
            eprintln!("error: timed out waiting for background tasks");
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    failed
}

fn exit_status(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

enum Outcome {
    Continue,
    Failed,
    Quit,
}

fn print_submission(session: &InterpreterSession, submission: Submission) -> Outcome {
    match submission {
        Submission::DeclarationAccepted { diagnostics } => print_diagnostics(&diagnostics),
        Submission::Answered { text } => println!("{text}"),
        Submission::Evaluated { value, diagnostics } => {
            print_evaluated_value(&value, &diagnostics, None);
        }
        Submission::Inspected { value, diagnostics } => {
            print_diagnostics(&diagnostics);
            let inspection = value.inspection(InspectionLimits::default());
            let type_hint = if inspection.type_truncated {
                " (summary; use #type <expression> for details)"
            } else {
                ""
            };
            println!(
                "type: {}{type_hint}\nvalue: {}",
                inspection.type_name, inspection.display
            );
        }
        Submission::Command(Command::Clear) => println!("session cleared"),
        Submission::Command(Command::Help) => println!("{}", ginterpreter::help(None)),
        Submission::Command(Command::Reset) => println!("session and running tasks reset"),
        Submission::Command(Command::Tasks) => print_task_summary(session),
        Submission::Command(Command::TaskStarted(task_id)) => {
            println!("started task {}", task_id.0);
        }
        Submission::Command(Command::TaskStopRequested(task_id)) => {
            println!("requested stop for task {}", task_id.0);
        }
        Submission::Command(Command::Quit) => return Outcome::Quit,
        Submission::Empty => {}
        Submission::Incomplete => {
            eprintln!("error: incomplete submission; finish the expression or declaration");
            return Outcome::Failed;
        }
        Submission::Rejected { diagnostics } | Submission::Cancelled { diagnostics } => {
            print_diagnostics(&diagnostics);
            print_condition_hint(&diagnostics, None);
            return Outcome::Failed;
        }
        Submission::UnknownCommand(command) => {
            eprintln!("unknown command `#{command}`; type #help for commands");
            return Outcome::Failed;
        }
    }
    Outcome::Continue
}

fn print_diagnostics(diagnostics: &[ginterpreter::InterpreterDiagnostic]) {
    for diagnostic in diagnostics {
        diagnostic
            .diagnostic()
            .print(diagnostic.source(), diagnostic.display_path());
    }
}

fn print_evaluated_value(
    value: &EvaluatedValue,
    diagnostics: &[ginterpreter::InterpreterDiagnostic],
    task: Option<TaskId>,
) {
    print_diagnostics(diagnostics);
    let prefix = task
        .map(|id| format!("task {} => ", id.0))
        .unwrap_or_default();
    println!("{prefix}{}", value.display());
}

fn print_condition_hint(diagnostics: &[ginterpreter::InterpreterDiagnostic], task: Option<TaskId>) {
    let count = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.frames().len())
        .max()
        .unwrap_or(0);
    if count > 0 {
        let selector = task.map(|id| format!(" {}", id.0)).unwrap_or_default();
        println!(
            "{count} evaluator frames captured; inspect with #frames{selector} or #frame 0{selector}"
        );
        println!("restarts: #retry{selector}, #abort{selector}");
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

fn process_task_results(session: &mut InterpreterSession) -> bool {
    let mut failed = false;
    for task in session.poll_task_results() {
        failed |= matches!(task.state, TaskState::Cancelled | TaskState::Failed);
        let state = match task.state {
            TaskState::Running => "running",
            TaskState::Cancelled => "cancelled",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
        };
        println!("task {} {state}: {}", task.id.0, task.expression);
        handle_task_result(task);
    }
    failed
}

fn handle_task_result(task: TaskResult) {
    match (task.state, task.submission) {
        (TaskState::Cancelled, submission) | (TaskState::Failed, submission) => {
            if let Submission::Rejected { diagnostics } | Submission::Cancelled { diagnostics } =
                submission
            {
                print_diagnostics(&diagnostics);
                print_condition_hint(&diagnostics, Some(task.id));
            } else {
                eprintln!("task {} ended unexpectedly", task.id.0);
            }
        }
        (TaskState::Completed, submission) => match submission {
            ginterpreter::Submission::Evaluated { value, diagnostics } => {
                print_evaluated_value(&value, &diagnostics, Some(task.id));
            }
            ginterpreter::Submission::Rejected { diagnostics } => {
                print_diagnostics(&diagnostics);
            }
            _ => {}
        },
        (TaskState::Running, _) => {}
    }
}
