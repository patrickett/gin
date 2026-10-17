use diagnostic::DiagnosticCode;
use diagnostic::code::{InterpreterCode, ProtocolCode};
use ginterpreter::{Command, InspectionLimits, InterpreterSession, Submission};
use serde::Deserialize;
use serde_json::{Value, json};

use super::response::Response;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Operation {
    Describe {},
    Queries {
        filter: Option<String>,
    },
    Help {
        topic: Option<String>,
    },
    Submit {
        source: String,
    },
    Inspect {
        expression: String,
        limits: Option<RequestedLimits>,
    },
    Scope {},
    Doc {
        symbol: String,
    },
    Search {
        text: String,
    },
    Complete {
        source: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestedLimits {
    depth_max: Option<u32>,
    nodes_max: Option<u32>,
    text_bytes_max: Option<u32>,
}

impl RequestedLimits {
    fn inspection(self) -> InspectionLimits {
        let defaults = InspectionLimits::default();
        InspectionLimits {
            depth_max: self.depth_max.unwrap_or(defaults.depth_max),
            nodes_max: self.nodes_max.unwrap_or(defaults.nodes_max),
            text_bytes_max: self.text_bytes_max.unwrap_or(defaults.text_bytes_max),
        }
    }
}

pub(super) fn parse(line: &[u8]) -> Result<(Value, Operation), Box<Response>> {
    let raw: Value = serde_json::from_slice(line).map_err(|error| {
        // jsonl-invalid-json
        Box::new(Response::error(
            Value::Null,
            DiagnosticCode::Protocol(ProtocolCode::InvalidJson),
            &error.to_string(),
        ))
    })?;
    let Value::Object(mut object) = raw else {
        // jsonl-invalid-request
        return Err(Box::new(Response::error(
            Value::Null,
            DiagnosticCode::Protocol(ProtocolCode::InvalidRequest),
            "request must be an object",
        )));
    };
    let id = object.remove("id").unwrap_or(Value::Null);
    let valid = match &id {
        Value::String(id) => id.len() <= 256,
        Value::Number(id) => id.is_i64() || id.is_u64(),
        _ => false,
    };
    if !valid {
        // jsonl-invalid-id
        return Err(Box::new(Response::error(
            Value::Null,
            DiagnosticCode::Protocol(ProtocolCode::InvalidId),
            "id must be an integer or a string of at most 256 bytes",
        )));
    }
    let operation = serde_json::from_value(Value::Object(object)).map_err(|error| {
        // jsonl-invalid-request
        Box::new(Response::error(
            id.clone(),
            DiagnosticCode::Protocol(ProtocolCode::InvalidRequest),
            &error.to_string(),
        ))
    })?;
    Ok((id, operation))
}

pub(super) fn handle(
    id: Value,
    operation: Operation,
    session: &mut InterpreterSession,
) -> (Response, bool) {
    let response = match operation {
        Operation::Describe {} => {
            Response::ok(id, "description", super::description::describe(session))
        }
        Operation::Queries { filter } => Response::ok(
            id,
            "queries",
            json!(ginterpreter::query_catalog(filter.as_deref())),
        ),
        Operation::Help { topic } => Response::ok(id, "help", help_data(topic.as_deref())),
        Operation::Submit { source } => {
            let submission = session.submit(&source);
            let quit = matches!(submission, Submission::Command(Command::Quit));
            return (Response::submission(id, submission, session), quit);
        }
        Operation::Inspect { expression, limits } => {
            let limits = limits.map(RequestedLimits::inspection).unwrap_or_default();
            Response::inspection(id, session.inspect(&expression), session, limits)
        }
        Operation::Scope {} => Response::ok(
            id,
            "scope",
            json!(
                session
                    .scope()
                    .iter()
                    .map(|entry| json!({"name":entry.name,"signature":entry.signature}))
                    .collect::<Vec<_>>()
            ),
        ),
        Operation::Doc { symbol } => documentation(id, &symbol, session),
        Operation::Search { text } => Response::ok(id, "search", json!(session.search(&text))),
        Operation::Complete { source } => {
            Response::ok(id, "completions", json!(session.completions(&source)))
        }
    };
    (response, false)
}

fn help_data(topic: Option<&str>) -> Value {
    let mut data = json!({"text": ginterpreter::help(topic)});
    if let Some(recipe) = topic.and_then(ginterpreter::language_recipe) {
        data["recipe"] = json!(recipe);
    }
    data
}

fn documentation(id: Value, symbol: &str, session: &InterpreterSession) -> Response {
    match session.documentation(symbol.trim()) {
        Some(documentation) => Response::ok(
            id,
            "documentation",
            json!({"symbol": symbol, "text": documentation}),
        ),
        // interpreter-unknown-symbol
        None => Response::error(
            id,
            DiagnosticCode::Interpreter(InterpreterCode::UnknownSymbol),
            "use search or scope to find available symbols",
        ),
    }
}
