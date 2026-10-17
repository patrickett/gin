use diagnostic::DiagnosticCode;
use diagnostic::Span;
use diagnostic::code::InterpreterCode;
use ginterpreter::{
    Command, EvaluatedValue, InspectionLimits, InterpreterDiagnostic, InterpreterFrame,
    InterpreterSession, Submission,
};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Serialize)]
pub(super) struct Response {
    pub id: Value,
    pub status: &'static str,
    pub kind: &'static str,
    pub data: Value,
    pub diagnostics: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<u64>,
}

impl Response {
    pub fn ok(id: Value, kind: &'static str, data: Value) -> Self {
        Self {
            id,
            status: "ok",
            kind,
            data,
            diagnostics: Vec::new(),
            event: None,
            task_id: None,
        }
    }

    pub fn error(id: Value, code: DiagnosticCode, message: &str) -> Self {
        let mut response = Self::ok(id, "protocol_error", Value::Null);
        response.status = "error";
        response
            .diagnostics
            .push(json!({"code":code.slug().as_str(),"message":message,"severity":"error"}));
        response
    }

    pub fn submission(id: Value, submission: Submission, session: &InterpreterSession) -> Self {
        match submission {
            Submission::DeclarationAccepted { diagnostics } => {
                Self::with_diagnostics(id, "declaration_accepted", Value::Null, diagnostics)
            }
            Submission::Evaluated { value, diagnostics } => Self::value(
                id,
                "evaluated",
                value,
                diagnostics,
                InspectionLimits::default(),
            ),
            Submission::Inspected { value, diagnostics } => Self::value(
                id,
                "inspected",
                value,
                diagnostics,
                InspectionLimits::default(),
            ),
            Submission::Answered { text } => Self::ok(id, "answered", json!({"text":text})),
            Submission::Command(command) => Self::ok(id, "command", command_data(command, session)),
            Submission::Empty => Self::ok(id, "empty", Value::Null),
            Submission::Incomplete => {
                // interpreter-incomplete
                let mut response = Self::error(
                    id,
                    DiagnosticCode::Interpreter(InterpreterCode::Incomplete),
                    "send a complete submission in the source field",
                );
                response.kind = "incomplete";
                response
            }
            Submission::Rejected { diagnostics } => Self::rejected(id, "rejected", diagnostics),
            Submission::Cancelled { diagnostics } => Self::rejected(id, "cancelled", diagnostics),
            Submission::UnknownCommand(command) => {
                // interpreter-unknown-command
                let mut response = Self::error(
                    id,
                    DiagnosticCode::Interpreter(InterpreterCode::UnknownCommand),
                    &format!("unknown command `#{command}`; use #help"),
                );
                response.kind = "unknown_command";
                response
            }
        }
    }

    pub fn inspection(
        id: Value,
        submission: Submission,
        session: &InterpreterSession,
        limits: InspectionLimits,
    ) -> Self {
        match submission {
            Submission::Inspected { value, diagnostics } => {
                Self::value(id, "inspected", value, diagnostics, limits)
            }
            submission => Self::submission(id, submission, session),
        }
    }

    fn value(
        id: Value,
        kind: &'static str,
        value: EvaluatedValue,
        diagnostics: Vec<InterpreterDiagnostic>,
        limits: InspectionLimits,
    ) -> Self {
        Self::with_diagnostics(
            id,
            kind,
            json!({"value": value.inspection(limits)}),
            diagnostics,
        )
    }

    fn with_diagnostics(
        id: Value,
        kind: &'static str,
        data: Value,
        diagnostics: Vec<InterpreterDiagnostic>,
    ) -> Self {
        let mut response = Self::ok(id, kind, data);
        response.diagnostics = diagnostics.iter().map(diagnostic_data).collect();
        response
    }

    fn rejected(id: Value, kind: &'static str, diagnostics: Vec<InterpreterDiagnostic>) -> Self {
        let mut response = Self::with_diagnostics(id, kind, Value::Null, diagnostics);
        response.status = "error";
        response
    }
}

fn command_data(command: Command, session: &InterpreterSession) -> Value {
    match command {
        Command::Clear => json!({"command":"clear"}),
        Command::Reset => json!({"command":"reset"}),
        Command::Help => json!({"command":"help","text":ginterpreter::help(None)}),
        Command::Quit => json!({"command":"quit"}),
        Command::Tasks => {
            let tasks: Vec<_> = session.task_summaries().iter().map(|task| {
                json!({"task_id": task.id.0, "state": "running", "expression": task.expression})
            }).collect();
            json!({"command": "tasks", "tasks": tasks})
        }
        Command::TaskStarted(id) => json!({"command":"task_started","task_id":id.0}),
        Command::TaskStopRequested(id) => json!({"command":"task_stop_requested","task_id":id.0}),
    }
}

fn span_data(span: Span) -> Value {
    json!({"start":span.start(),"end":span.end()})
}

fn diagnostic_data(diagnostic: &InterpreterDiagnostic) -> Value {
    let detail = diagnostic.diagnostic();
    let (line, column) = diagnostic::source_position(diagnostic.source(), detail.span.start());
    let severity = match detail.category {
        diagnostic::Category::Flaw => "error",
        diagnostic::Category::Warn => "warning",
        diagnostic::Category::Help => "help",
    };
    let related: Vec<_> = detail
        .related()
        .iter()
        .map(|related| json!({"span": span_data(related.span), "label": related.label}))
        .collect();
    let frames: Vec<_> = diagnostic.frames().iter().map(frame_data).collect();
    json!({
        "code": detail.code.slug().as_str(), "message": detail.message, "severity": severity,
        "path": diagnostic.display_path(), "source": diagnostic.source(),
        "span": span_data(detail.span), "line": line, "column": column,
        "help": detail.help, "help_on_span": detail.help_on_span, "args": detail.args(),
        "related": related, "frames": frames
    })
}

fn frame_data(frame: &InterpreterFrame) -> Value {
    let locals: Vec<_> = frame.locals.iter().map(local_data).collect();
    json!({
        "function": frame.function,
        "path": frame.display_path,
        "source": frame.source.as_ref(),
        "span": span_data(frame.span),
        "locals": locals
    })
}

fn local_data(local: &typecheck::EvalLocal) -> Value {
    let result = match &local.value {
        Ok(value) => {
            let value = EvaluatedValue::from_constant(value.clone());
            json!({"value": value.inspection(InspectionLimits::default())})
        }
        Err(diagnostic) => {
            json!({"error": {"code": diagnostic.code.slug().as_str(), "message": diagnostic.message}})
        }
    };
    json!({"name": local.name.as_str(), "is_reference": local.is_reference, "result": result})
}
