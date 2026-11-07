//! The JSONL protocol: one JSON request per line in, one JSON response per line out.
//!
//! Operations are spelled with a `$` (`{"op":"$check",...}`), as are the `kind`s of their
//! responses; see `commands.rs`.
//!
//! Every response is `{"v":1,"id":..,"status":"ok"|"error","kind":..,"data":..,"diagnostics":[..]}`.
//! `status` says whether the request was understood and served; whether the *program* is
//! right is in `data` (`complete`, `accepted`, `known`) and `diagnostics`. Output is
//! deterministic: keys are sorted, diagnostics are ordered by position.

use std::io::{BufRead, Write};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::commands::JSONL_OPS;
use crate::check::{Checked, check_package, check_text, read_text};
use crate::flaw::Flaw;
use crate::session::{EvalOutcome, EvalWhat, Session, Target, arg_of};

pub const PROTOCOL: &str = "ginterpreter/1";
pub const REQUEST_BYTES_MAX: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Operation {
    #[serde(rename = "$describe")]
    Describe {},
    #[serde(rename = "$check")]
    Check { source: Option<String>, path: Option<String> },
    #[serde(rename = "$holes")]
    Holes { source: Option<String>, path: Option<String> },
    #[serde(rename = "$eval")]
    Eval {
        expression: Option<String>,
        call: Option<String>,
        #[serde(default)]
        args: Vec<Value>,
        #[serde(default)]
        entry: bool,
        source: Option<String>,
        path: Option<String>,
    },
    #[serde(rename = "$declare")]
    Declare { source: String },
    #[serde(rename = "$remove")]
    Remove { name: String },
    #[serde(rename = "$scope")]
    Scope {},
    #[serde(rename = "$inspect")]
    Inspect { name: String },
    #[serde(rename = "$image")]
    Image {},
    #[serde(rename = "$save")]
    Save { path: String },
    #[serde(rename = "$restore")]
    Restore {
        path: String,
        #[serde(default)]
        verify: bool,
    },
    #[serde(rename = "$load")]
    Load { path: String },
    #[serde(rename = "$reset")]
    Reset {},
    #[serde(rename = "$quit")]
    Quit {},
}

pub struct Response {
    pub id: Value,
    pub status: &'static str,
    pub kind: &'static str,
    pub data: Value,
    pub diagnostics: Vec<Value>,
}

impl Response {
    fn ok(id: Value, kind: &'static str, data: Value) -> Self {
        Self { id, status: "ok", kind, data, diagnostics: Vec::new() }
    }

    fn with(mut self, flaws: &[&Flaw]) -> Self {
        self.diagnostics = flaws.iter().map(|f| f.json()).collect();
        self
    }

    pub fn error(id: Value, code: &str, message: impl Into<String>) -> Self {
        Self {
            id,
            status: "error",
            kind: "protocol_error",
            data: Value::Null,
            diagnostics: vec![json!({"code": code, "severity": "error", "message": message.into()})],
        }
    }

    pub fn json(&self) -> Value {
        json!({"v": 1, "id": self.id, "status": self.status, "kind": self.kind, "data": self.data, "diagnostics": self.diagnostics})
    }
}

/// Handle one request line. Returns the response and whether the session should end.
pub fn handle_line(session: &mut Session, line: &[u8]) -> (Response, bool) {
    let raw: Value = match serde_json::from_slice(line) {
        Ok(raw) => raw,
        Err(error) => return (Response::error(Value::Null, "jsonl-invalid-json", error.to_string()), false),
    };
    let Value::Object(mut object) = raw else {
        return (Response::error(Value::Null, "jsonl-invalid-request", "request must be an object"), false);
    };
    let id = object.remove("id").unwrap_or(Value::Null);
    let id_valid = match &id {
        Value::String(s) => s.len() <= 256,
        Value::Number(n) => n.is_i64() || n.is_u64(),
        _ => false,
    };
    if !id_valid {
        return (Response::error(Value::Null, "jsonl-invalid-id", "id must be an integer or a string of at most 256 bytes"), false);
    }
    match object.get("op").and_then(Value::as_str) {
        None => return (Response::error(id, "jsonl-invalid-request", "request needs a string `op`"), false),
        Some(op) if !op.strip_prefix('$').is_some_and(|bare| JSONL_OPS.contains(&bare)) => {
            // Operations are spelled with a `$`; the bare word is the one mistake worth naming.
            let hint = crate::commands::suggestion(op).map(|s| format!("; did you mean `{s}`?")).unwrap_or_default();
            let all: Vec<String> = JSONL_OPS.iter().map(|o| format!("${o}")).collect();
            return (Response::error(id, "jsonl-unknown-op", format!("unknown op `{op}`{hint}; operations: {}", all.join(", "))), false);
        }
        Some(_) => {}
    }
    let operation: Operation = match serde_json::from_value(Value::Object(object)) {
        Ok(operation) => operation,
        Err(error) => return (Response::error(id, "jsonl-invalid-request", error.to_string()), false),
    };
    let quit = matches!(operation, Operation::Quit {});
    (dispatch(session, id, operation), quit)
}

fn checked_of(source: Option<String>, path: Option<String>, session: &Session) -> Result<Checked, String> {
    match (source, path) {
        (Some(_), Some(_)) => Err("give either `source` or `path`, not both".to_owned()),
        (Some(text), None) => Ok(check_text("<source>", &text)),
        (None, Some(path)) => {
            let path = Path::new(&path);
            if path.is_dir() {
                Ok(check_package(path))
            } else {
                read_text(path).map(|text| check_text(&path.display().to_string(), &text))
            }
        }
        (None, None) => Ok(session.check()),
    }
}

fn dispatch(session: &mut Session, id: Value, operation: Operation) -> Response {
    match operation {
        Operation::Describe {} => Response::ok(id, "$describe", describe(session)),
        Operation::Check { source, path } => match checked_of(source, path, session) {
            Err(message) => Response::error(id, "jsonl-invalid-request", message),
            Ok(checked) => {
                let all: Vec<&Flaw> = checked.all().collect();
                let data = json!({
                    "complete": checked.flaws.is_empty(),
                    "flaw_count": checked.flaws.len(),
                    "warning_count": checked.warnings.len(),
                    "hole_count": checked.holes.len(),
                    // The checker stops at its first failure; later flaws were found by setting
                    // the failing declaration aside (see `knock_on_of`).
                    "note": "flaws after the first were found with earlier failing declarations set aside",
                });
                Response::ok(id, "$check", data).with(&all)
            }
        },
        Operation::Holes { source, path } => match checked_of(source, path, session) {
            Err(message) => Response::error(id, "jsonl-invalid-request", message),
            Ok(checked) => {
                let all: Vec<&Flaw> = checked.flaws.iter().collect();
                let data = json!({
                    // Holes of declarations that did not check cannot be listed.
                    "complete": checked.flaws.is_empty(),
                    "holes": checked.holes.iter().map(|h| h.json()).collect::<Vec<_>>(),
                });
                Response::ok(id, "$holes", data).with(&all)
            }
        },
        Operation::Eval { expression, call, args, entry, source, path } => {
            let what = match (expression, call, entry) {
                (Some(e), None, false) => EvalWhat::Expression(e),
                (None, Some(function), false) => {
                    let mut arguments = Vec::new();
                    for arg in &args {
                        match arg_of(arg) {
                            Ok(arg) => arguments.push(arg),
                            Err(message) => return Response::error(id, "jsonl-invalid-request", message),
                        }
                    }
                    EvalWhat::Call { function, arguments }
                }
                (None, None, true) => EvalWhat::Entry,
                _ => return Response::error(id, "jsonl-invalid-request", "give exactly one of `expression`, `call` (with `args`), or `entry`"),
            };
            let target = match (source, path) {
                (Some(_), Some(_)) => return Response::error(id, "jsonl-invalid-request", "give either `source` or `path`, not both"),
                (Some(text), None) => Target::Text(text),
                (None, Some(path)) => Target::Package(path.into()),
                (None, None) => Target::Session,
            };
            match session.eval(&target, &what) {
                EvalOutcome::Value { json: value, display, ty } => {
                    Response::ok(id, "$eval", json!({"known": true, "value": value, "display": display, "type": ty}))
                }
                EvalOutcome::Unknown { reason, .. } => Response::ok(id, "$eval", json!({"known": false, "reason": reason})),
                EvalOutcome::Flawed(flaws) => {
                    let refs: Vec<&Flaw> = flaws.iter().collect();
                    let reason = json!({"kind": "flawed", "detail": "the program or expression has flaws, so nothing was run"});
                    Response::ok(id, "$eval", json!({"known": false, "reason": reason})).with(&refs)
                }
            }
        }
        Operation::Declare { source } => declared(id, session.declare(&source), "$declare"),
        Operation::Load { path } => {
            let path = Path::new(&path);
            if path.is_dir() {
                return Response::error(id, "jsonl-invalid-request", "load takes a .gin file; packages are used with `path` in check, holes and eval");
            }
            match read_text(path) {
                Ok(text) => declared(id, session.load_text(&text), "$load"),
                Err(message) => Response::error(id, "jsonl-invalid-request", message),
            }
        }
        Operation::Remove { name } => {
            let removed = session.remove(&name);
            Response::ok(id, "$remove", json!({"removed": removed}))
        }
        Operation::Scope {} => {
            let entries: Vec<Value> = session.scope().iter().map(|e| json!({"kind": e.kind, "name": e.name, "signature": e.signature})).collect();
            Response::ok(id, "$scope", json!({"entries": entries}))
        }
        Operation::Inspect { name } => {
            let data = session.inspect(&name);
            Response::ok(id, "$inspect", data)
        }
        Operation::Save { path } => match session.save(Path::new(&path)) {
            Ok(data) => Response::ok(id, "$save", data),
            Err(notice) => Response::ok(id, "$save", json!({"saved": false, "notices": [notice]})),
        },
        Operation::Restore { path, verify } => Response::ok(id, "$restore", session.restore(Path::new(&path), verify)),
        Operation::Image {} => {
            let (data, _) = session.image();
            Response::ok(id, "$image", data)
        }
        Operation::Reset {} => {
            session.reset();
            Response::ok(id, "$reset", Value::Null)
        }
        Operation::Quit {} => Response::ok(id, "$quit", Value::Null),
    }
}

fn declared(id: Value, result: Result<crate::session::Declared, Vec<Flaw>>, kind: &'static str) -> Response {
    match result {
        Err(flaws) => {
            let refs: Vec<&Flaw> = flaws.iter().collect();
            Response::ok(id, kind, json!({"accepted": false, "added": [], "replaced": [], "image": null})).with(&refs)
        }
        Ok(declared) => {
            let all: Vec<&Flaw> = declared.flaws.iter().chain(&declared.warnings).collect();
            let data = json!({"accepted": declared.accepted, "added": declared.added, "replaced": declared.replaced,
                "image": {"revision": declared.revision, "changed": declared.changed, "kept": declared.kept}});
            Response::ok(id, kind, data).with(&all)
        }
    }
}

fn describe(session: &Session) -> Value {
    json!({
        "protocol": PROTOCOL,
        "version": 1,
        "declarations": session.declaration_count(),
        "operations": [
            {"name": "$describe", "args": {}},
            {"name": "$check", "args": {"source": "string?", "path": "string? (file or package directory)"}, "note": "no source or path checks the session"},
            {"name": "$holes", "args": {"source": "string?", "path": "string?"}},
            {"name": "$eval", "args": {"expression": "string?", "call": "string?", "args": "[integer|string|null]", "entry": "bool", "source": "string?", "path": "string?"}, "note": "exactly one of expression, call, entry; null args are unknown inputs"},
            {"name": "$declare", "args": {"source": "string"}, "note": "accepted only if the accumulated module still checks"},
            {"name": "$remove", "args": {"name": "#string"}},
            {"name": "$scope", "args": {}},
            {"name": "$inspect", "args": {"name": "string (a type, alias, constant or function; `Pair(Word)` for an application)"}, "note": "shape, size and layout from the checker and the lowering; unavailable parts carry a typed reason"},
            {"name": "$save", "args": {"path": "string"}, "note": "write the image (declarations and derived state with closure hashes) to a snapshot file"},
            {"name": "$restore", "args": {"path": "string", "verify": "bool (default false): re-derive every kept entry and compare"}, "note": "re-check a snapshot's declarations; derived state whose closure hash still matches is kept; a stale or corrupt snapshot gives typed notices and changes nothing"},
            {"name": "$image", "args": {}, "note": "definitions, what each uses and affects, folded constants and target facts, instances"},
            {"name": "$load", "args": {"path": "string (.gin file)"}},
            {"name": "$reset", "args": {}},
            {"name": "$quit", "args": {}},
        ],
        "flaw_fields": ["code", "severity", "stage", "message", "span", "construct", "claim", "facts", "knock_on_of"],
        "limits": {"request_bytes_max": REQUEST_BYTES_MAX},
    })
}

/// Serve requests from `input` until it ends or a `quit`.
pub fn serve(session: &mut Session, mut input: impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = std::io::Read::take(&mut input, REQUEST_BYTES_MAX as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        let too_long = line.len() > REQUEST_BYTES_MAX && !line.ends_with(b"\n");
        if too_long {
            // Drop the rest of the line.
            let mut sink = Vec::new();
            loop {
                sink.clear();
                let n = std::io::Read::take(&mut input, 65536).read_until(b'\n', &mut sink)?;
                if n == 0 || sink.ends_with(b"\n") {
                    break;
                }
            }
            write_response(output, &Response::error(Value::Null, "jsonl-request-too-large", format!("request exceeds {REQUEST_BYTES_MAX} bytes")))?;
            continue;
        }
        let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
        if trimmed.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let (response, quit) = handle_line(session, trimmed);
        write_response(output, &response)?;
        if quit {
            return Ok(());
        }
    }
}

fn write_response(output: &mut impl Write, response: &Response) -> std::io::Result<()> {
    serde_json::to_writer(&mut *output, &response.json())?;
    output.write_all(b"\n")?;
    output.flush()
}
