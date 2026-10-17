use diagnostic::DiagnosticCode;
use diagnostic::code::{InterpreterCode, ProtocolCode};
mod description;
mod request;
mod response;

use ginterpreter::{InterpreterSession, TaskId};
use response::Response;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub(super) const REQUEST_BYTES_MAX: usize = 1024 * 1024;
const INPUT_QUEUE_MAX: usize = 8;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

enum Input {
    Line(Vec<u8>),
    TooLong,
    Error(io::Error),
}

pub(super) fn run(session: &mut InterpreterSession) -> ExitCode {
    let input = spawn_input();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match serve(session, &input, &mut output) {
        Ok(failed) => super::exit_status(failed),
        Err(error) => {
            session.clear_tasks();
            eprintln!("JSONL output failed: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(super) fn startup_error(message: &str) -> ExitCode {
    // interpreter-context-unavailable
    let response = Response::error(
        Value::Null,
        DiagnosticCode::Interpreter(InterpreterCode::ContextUnavailable),
        message,
    );
    if let Err(error) = write_response(&mut io::stdout().lock(), &response) {
        eprintln!("JSONL output failed: {error}");
    }
    ExitCode::FAILURE
}

fn serve(
    session: &mut InterpreterSession,
    input: &Receiver<Input>,
    output: &mut impl Write,
) -> io::Result<bool> {
    let mut failed = false;
    let mut task_requests = HashMap::new();
    loop {
        failed |= poll_tasks(session, &mut task_requests, output)?;
        let incoming = match input.recv_timeout(POLL_INTERVAL) {
            Ok(incoming) => incoming,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let (response, quit) = input_response(incoming, session);
        failed |= response.status == "error";
        if response.data["command"] == "task_started" {
            let task_id = response.data["task_id"]
                .as_u64()
                .expect("task id is an integer");
            task_requests.insert(TaskId(task_id), response.id.clone());
        }
        if response.data["command"] == "reset" {
            task_requests.clear();
        }
        write_response(output, &response)?;
        if quit {
            session.clear_tasks();
            return Ok(failed);
        }
    }
    failed |= drain_tasks(session, &mut task_requests, output)?;
    Ok(failed)
}

fn input_response(input: Input, session: &mut InterpreterSession) -> (Response, bool) {
    match input {
        Input::Line(line) => match request::parse(&line) {
            Ok((id, operation)) => request::handle(id, operation, session),
            Err(response) => (*response, false),
        },
        Input::TooLong => (
            // jsonl-request-too-large
            Response::error(
                Value::Null,
                DiagnosticCode::Protocol(ProtocolCode::RequestTooLarge),
                "request exceeds 1048576 bytes",
            ),
            false,
        ),
        Input::Error(error) => (
            // jsonl-input-failed
            Response::error(
                Value::Null,
                DiagnosticCode::Protocol(ProtocolCode::InputFailed),
                &error.to_string(),
            ),
            true,
        ),
    }
}

fn poll_tasks(
    session: &mut InterpreterSession,
    requests: &mut HashMap<TaskId, Value>,
    output: &mut impl Write,
) -> io::Result<bool> {
    let mut failed = false;
    for task in session.poll_task_results() {
        let id = requests.remove(&task.id).unwrap_or(Value::Null);
        let mut response = Response::submission(id, task.submission, session);
        response.event = Some("task");
        response.task_id = Some(task.id.0);
        failed |= response.status == "error";
        write_response(output, &response)?;
    }
    Ok(failed)
}

fn drain_tasks(
    session: &mut InterpreterSession,
    requests: &mut HashMap<TaskId, Value>,
    output: &mut impl Write,
) -> io::Result<bool> {
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    let mut failed = false;
    while !session.task_summaries().is_empty() {
        failed |= poll_tasks(session, requests, output)?;
        if Instant::now() >= deadline {
            session.clear_tasks();
            write_response(
                output,
                // jsonl-task-timeout
                &Response::error(
                    Value::Null,
                    DiagnosticCode::Protocol(ProtocolCode::TaskTimeout),
                    "timed out waiting for background tasks",
                ),
            )?;
            return Ok(true);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    Ok(failed)
}

fn write_response(output: &mut impl Write, response: &Response) -> io::Result<()> {
    serde_json::to_writer(&mut *output, response)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn spawn_input() -> Receiver<Input> {
    let (sender, receiver) = mpsc::sync_channel(INPUT_QUEUE_MAX);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        loop {
            let message = match read_request(&mut input) {
                Ok(Some(input)) => input,
                Ok(None) => return,
                Err(error) => {
                    let _ = sender.send(Input::Error(error));
                    return;
                }
            };
            if sender.send(message).is_err() {
                return;
            }
        }
    });
    receiver
}

fn read_request(input: &mut impl BufRead) -> io::Result<Option<Input>> {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if too_long {
                Some(Input::TooLong)
            } else if line.is_empty() {
                None
            } else {
                Some(Input::Line(line))
            });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let length = newline.unwrap_or(buffer.len());
        if !too_long {
            if length <= REQUEST_BYTES_MAX - line.len() {
                line.extend_from_slice(&buffer[..length]);
            } else {
                too_long = true;
                line.clear();
            }
        }
        input.consume(length + usize::from(newline.is_some()));
        if newline.is_some() {
            return Ok(Some(if too_long {
                Input::TooLong
            } else {
                Input::Line(line)
            }));
        }
    }
}
