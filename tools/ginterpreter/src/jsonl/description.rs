use ginterpreter::{InspectionLimits, InterpreterSession};
use serde_json::{Value, json};

pub(super) fn describe(session: &InterpreterSession) -> Value {
    let context = session.context();
    let defaults = InspectionLimits::default();
    json!({
        "protocol_version": 1,
        "interpreter_version": env!("CARGO_PKG_VERSION"),
        "operations": operations(),
        "capabilities": ginterpreter::capabilities(),
        "language_topics": language_topics(),
        "context": {
            "isolated": context.is_isolated(),
            "package_root": context.package().root().to_string_lossy(),
            "target": context.target().triple_raw().unwrap_or("library")
        },
        "limits": {
            "request_bytes": super::REQUEST_BYTES_MAX,
            "id_bytes": 256,
            "task_drain_seconds": 30,
            "inspection_defaults": {
                "depth_max": defaults.depth_max,
                "nodes_max": defaults.nodes_max,
                "text_bytes_max": defaults.text_bytes_max
            },
            "inspection_caps": {"depth_max": 32, "nodes_max": 4096, "text_bytes_max": 65536}
        },
        "conventions": conventions(),
        "examples": [
            {"id": 1, "op": "queries", "filter": "size"},
            {"id": 2, "op": "submit", "source": "#size(@())"},
            {"id": 3, "op": "inspect", "expression": "1 < 2"},
            {"id": 4, "op": "inspect", "expression": "#target", "limits": {"depth_max": 2}}
        ]
    })
}

fn operations() -> Vec<Value> {
    [
        ("describe", &[][..], &[][..],
         "Discover protocol operations, Gin commands and compiler queries."),
        ("queries", &[], &["filter"],
         "List compiler queries with syntax, prerequisites and runnable examples."),
        ("help", &[], &["topic"],
         "Read help for a language recipe, command or compiler query."),
        ("submit", &["source"], &[],
         "Submit complete Gin code or a #command in the current session."),
        ("inspect", &["expression"], &["limits"],
         "Evaluate and inspect a value without committing evaluator state."),
        ("scope", &[], &[],
         "List session declarations, package root declarations and imports."),
        ("doc", &["symbol"], &[],
         "Read a declaration's signature and documentation."),
        ("search", &["text"], &[],
         "Search symbol names, documentation and interpreter capabilities."),
        ("complete", &["source"], &[],
         "Get completion candidates at the end of the supplied source."),
    ]
    .into_iter()
    .map(|(name, required, optional, summary)| {
        json!({"name": name, "required": required, "optional": optional, "summary": summary})
    })
    .collect()
}

fn language_topics() -> Vec<Value> {
    ginterpreter::language_recipes()
        .iter()
        .map(|recipe| {
            json!({
                "name": recipe.name, "title": recipe.title,
                "aliases": recipe.aliases, "summary": recipe.summary
            })
        })
        .collect()
}

fn conventions() -> Value {
    json!({
        "request_id": "Every request requires an integer or string id; responses echo it.",
        "responses": concat!(
            "Every response contains id, status (ok/error), kind, data, and diagnostics. ",
            "Declaration acceptance and value evaluation have distinct kinds. ",
            "Failed requests preserve accepted declarations and the stream remains usable. ",
            "Background task results additionally contain event=task and task_id."
        ),
        "source": concat!(
            "One JSON line per complete request; ",
            "encode multiline Gin source with JSON escapes."
        ),
        "language_help": concat!(
            "Use help with a name from language_topics. data.recipe contains runnable steps ",
            "and separately labeled intentional rejections. Submit each step.source in order."
        ),
        "spans": concat!(
            "UTF-8 byte offsets into each diagnostic's source, end exclusive; ",
            "line and column are 1-based Unicode scalar positions."
        ),
        "values": concat!(
            "Value nodes form a flat tree: node 0 is the root; children contain node indexes. ",
            "Numbers are strings to preserve precision. truncated flags identify omitted data; ",
            "type_truncated marks a type summary; submit #type <expression> for type details. ",
            "total_children counts all original children. ",
            "display is human-readable text, not executable Gin source."
        ),
        "inspection": concat!(
            "Optional limits set depth_max, nodes_max and text_bytes_max; ",
            "unspecified limits use defaults, supplied limits clamp to caps (nodes minimum 1). ",
            "Inspect a subexpression or increase limits for more value detail. ",
            "Deliberate type, domain and result-family summaries remain truncated at any limit."
        ),
        "tasks": "submit #run returns a task id; event=task messages echo its starting request id.",
        "exit": concat!(
            "EOF drains tasks for up to 30 seconds; submit #quit cancels tasks. ",
            "Any failed request or task makes exit nonzero."
        )
    })
}
