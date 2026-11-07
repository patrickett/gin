//! `interpreter run <file.gin|package-dir>` runs the `#entry` function;
//! `interpreter call <file.gin|package-dir> <function> [integer arguments...]` runs one.
//! The exit status of `run` is the entry's integer result, as `ginc --emit exe` would give.

use std::path::Path;
use std::process::ExitCode;

use interpreter::{Interpreter, Program, Value, with_big_stack};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    with_big_stack(move || go(&arguments))
}

fn go(arguments: &[String]) -> ExitCode {
    let (Some(command), Some(path)) = (arguments.first(), arguments.get(1)) else {
        eprintln!("usage: interpreter run <path> | call <path> <function> [integers...]");
        return ExitCode::from(2);
    };
    let path = Path::new(path);
    let program = if path.is_dir() {
        Program::from_package(path)
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| interpreter::LoadError::Package(e.to_string()))
            .and_then(|text| Program::from_source(&text))
    };
    let program = match program {
        Ok(program) => program,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(2);
        }
    };
    let mut interpreter = Interpreter::new(&program);
    let outcome = match command.as_str() {
        "run" => interpreter.run_entry(),
        "call" => {
            let Some(name) = arguments.get(2) else {
                eprintln!("call needs a function name");
                return ExitCode::from(2);
            };
            let values: Result<Vec<Value>, _> = arguments[3..]
                .iter()
                .map(|text| text.parse::<i128>().map(Value::integer))
                .collect();
            match values {
                Ok(values) => interpreter.call(name, &[], &values),
                Err(error) => {
                    eprintln!("bad integer argument: {error}");
                    return ExitCode::from(2);
                }
            }
        }
        other => {
            eprintln!("unknown command `{other}`");
            return ExitCode::from(2);
        }
    };
    match outcome {
        Ok(outcome) => {
            println!("{}", program.render(&outcome.value));
            for value in &outcome.mutated {
                println!("  mut -> {}", program.render(value));
            }
            match (command.as_str(), &outcome.value) {
                ("run", Value::Integer(n)) => ExitCode::from((n.as_i128() & 0xff) as u8),
                _ => ExitCode::SUCCESS,
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
