//! `ginterpreter [--jsonl | --eval <source>...] [path]`: a Gin session. Without `--jsonl`, a REPL;
//! with it, one JSON request per line on standard input and one JSON response per line on standard
//! output. With `--eval`, each argument is one whole submission and the process exits.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use ginterpreter::session::Session;
use ginterpreter::{jsonl, repl};

#[derive(Parser)]
#[command(version, about = "Check Gin code, list its holes, and evaluate what is known at compile time")]
struct Args {
    /// Speak the JSONL protocol on stdio instead of the REPL.
    #[arg(long, conflicts_with = "eval")]
    jsonl: bool,
    /// Submit one complete declaration or expression and exit; repeat to share a session. A
    /// submission with a flaw is reported and the exit status is nonzero, but later ones still run.
    #[arg(long = "eval", value_name = "SOURCE", allow_hyphen_values = true)]
    eval: Vec<String>,
    /// A .gin file to load into the session first.
    path: Option<PathBuf>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    // Recorded bodies are walked recursively; give the whole session a deep stack.
    interpreter::with_big_stack(move || {
        let mut session = Session::default();
        if let Some(path) = &args.path {
            match ginterpreter::check::read_text(path).map(|t| session.load_text(&t)) {
                Ok(Ok(d)) if d.accepted => {}
                Ok(Ok(d)) => {
                    d.flaws.iter().for_each(|f| f.print());
                    return ExitCode::FAILURE;
                }
                Ok(Err(flaws)) => {
                    flaws.iter().for_each(|f| f.print());
                    return ExitCode::FAILURE;
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    return ExitCode::FAILURE;
                }
            }
        }
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        if !args.eval.is_empty() {
            let result = repl::run_batch(&mut session, &args.eval, &mut out);
            let _ = out.flush();
            return match result {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    eprintln!("error: {error}");
                    ExitCode::FAILURE
                }
            };
        }
        let result = if args.jsonl {
            jsonl::serve(&mut session, std::io::stdin().lock(), &mut out)
        } else {
            repl::run(&mut session, &mut out)
        };
        let _ = out.flush();
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        }
    })
}
