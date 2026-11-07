//! The session: an accumulated module that declarations extend or replace, and the operations
//! over it. Everything here is data; `jsonl.rs` and `repl.rs` only present it.

use std::path::Path;

use interpreter::{Error, Interpreter, Program, Value};
use serde_json::{Value as Json, json};
use typecheck::{CheckedSourceBodyKind, CheckedSourceModule};

use crate::check::{Checked, SESSION_FILE, check_package, check_text, show_type};
use crate::image::Image;
use crate::flaw::Flaw;
use crate::source::{Split, split};

/// What a declaration submission did.
pub struct Declared {
    pub accepted: bool,
    /// Definitions of the image whose own text or dependencies changed (their derived state is
    /// dropped), and those that were not touched (it is kept).
    pub changed: Vec<String>,
    pub kept: Vec<String>,
    pub revision: u64,
    pub added: Vec<String>,
    pub replaced: Vec<String>,
    pub flaws: Vec<Flaw>,
    pub warnings: Vec<Flaw>,
}

/// What to evaluate against.
pub enum Target {
    Session,
    Text(String),
    Package(std::path::PathBuf),
}

pub enum Arg {
    Known(Value),
    Unknown,
}

pub enum EvalWhat {
    Expression(String),
    Call { function: String, arguments: Vec<Arg> },
    /// The `#entry` function.
    Entry,
}

pub enum EvalOutcome {
    Value { json: Json, display: String, ty: String },
    /// Not known at compile time, and exactly why.
    Unknown { reason: Json, text: String },
    /// The program (or the expression) has flaws; nothing was run.
    Flawed(Vec<Flaw>),
}

pub struct ScopeEntry {
    pub kind: &'static str,
    pub name: String,
    pub signature: String,
}

#[derive(Default)]
pub struct Session {
    /// (key, text) in order.
    pub(crate) decls: Vec<(String, String)>,
    /// The flaws that rejected the last declaration, for `:flaws`.
    pub last_flaws: Vec<Flaw>,
    counter: u64,
    pub(crate) revision: u64,
    /// The check of the accumulated module, kept between operations until a declaration
    /// changes it.
    pub(crate) current: Option<Checked>,
    /// Derived state per definition name (inspected layouts and values), with the closure hash
    /// of the definition when it was computed; valid while that hash is unchanged.
    pub(crate) cache: std::collections::HashMap<String, (u64, Json)>,
}

const WIDE: &str = "in -57896044618658097711785492504343953926634992332820282019728792003956564819968 to 57896044618658097711785492504343953926634992332820282019728792003956564819967";

impl Session {
    pub fn source(&self) -> String {
        let mut out = String::new();
        for (_, text) in &self.decls {
            out.push_str(text.trim_end());
            out.push_str("\n\n");
        }
        out
    }

    pub fn reset(&mut self) {
        self.decls.clear();
        self.last_flaws.clear();
        self.current = None;
        self.cache.clear();
        self.revision += 1;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn ensure_current_pub(&mut self) {
        self.ensure_current();
    }

    fn ensure_current(&mut self) {
        if self.current.is_none() {
            self.current = Some(check_text(SESSION_FILE, &self.source()));
        }
    }

    /// The image of the accumulated module: dependencies and closure hashes.
    pub fn image(&mut self) -> (Json, u64) {
        self.ensure_current();
        let checked = self.current.take().expect("current");
        let image = Image::build(&checked);
        let kept = |i: usize| Some(self.cache.values().any(|(h, _)| *h == image.closure_of_name(&image.defs[i].name)));
        let json = crate::inspect::image_json(&checked, &image, &kept, self.revision);
        self.current = Some(checked);
        (json, self.revision)
    }

    /// Inspect a name: a type, alias, constant or function. Derived results are kept per
    /// definition and reused while neither it nor anything it uses has changed.
    pub fn inspect(&mut self, name: &str) -> Json {
        self.ensure_current();
        let checked = self.current.take().expect("current");
        let image = Image::build(&checked);
        let head = name.split('(').next().unwrap_or(name).trim().to_owned();
        let hash = image.closure_of_name(&head);
        let applied = name.contains('(');
        let key = name.to_owned();
        let known = !image.find(&head).is_empty();
        let mut out = match self.cache.get(&key) {
            Some((h, cached)) if *h == hash && known => {
                let mut cached = cached.clone();
                cached["cached"] = json!(true);
                cached
            }
            _ => {
                let mut fresh = derive(&checked, name);
                if known && fresh["found"] == true {
                    self.cache.insert(key, (hash, fresh.clone()));
                }
                fresh["cached"] = json!(false);
                fresh
            }
        };
        let _ = applied;
        if known {
            let indexes = image.find(&head);
            out["image"] = json!({
                "uses": indexes.iter().flat_map(|i| image.defs[*i].uses.keys().map(|t| image.defs[*t].key.clone())).collect::<Vec<_>>(),
                "used_by": indexes.iter().flat_map(|i| image.used_by(*i).into_iter().map(|u| image.defs[u].key.clone())).collect::<Vec<_>>(),
                "affects": indexes.iter().flat_map(|i| image.affects(*i).into_iter().map(|u| image.defs[u].key.clone())).collect::<Vec<_>>(),
            });
        }
        out["revision"] = json!(self.revision);
        self.current = Some(checked);
        out
    }

    pub fn declaration_count(&self) -> usize {
        self.decls.len()
    }

    /// Check the accumulated module.
    pub fn check(&self) -> Checked {
        check_text(SESSION_FILE, &self.source())
    }

    /// Add or replace declarations. Accepted only if the accumulated module still checks; a
    /// rejected submission leaves the session as it was.
    pub fn declare(&mut self, source: &str) -> Result<Declared, Vec<Flaw>> {
        let parts = match split(source) {
            Ok(Split::Declarations(parts)) => parts,
            Ok(Split::Empty) => Vec::new(),
            Ok(Split::Expression(_)) => {
                return Err(vec![message_flaw("a declaration was expected, found an expression; use eval")]);
            }
            Err(diagnostics) => {
                let text: std::sync::Arc<str> = std::sync::Arc::from(source);
                return Err(diagnostics.iter().map(|d| crate::flaw::from_diagnostic("parse", d, "<submission>", &text, &[])).collect());
            }
        };
        let mut next = self.decls.clone();
        let (mut added, mut replaced) = (Vec::new(), Vec::new());
        for (decl, text) in parts {
            match next.iter_mut().find(|(key, _)| *key == decl.key) {
                Some(slot) => {
                    slot.1 = text;
                    replaced.push(decl.key.clone());
                }
                None => {
                    next.push((decl.key.clone(), text));
                    added.push(decl.key.clone());
                }
            }
        }
        let joined: String = next.iter().map(|(_, t)| format!("{}\n\n", t.trim_end())).collect();
        let checked = check_text(SESSION_FILE, &joined);
        let unchanged = || (Vec::new(), Vec::new());
        if !checked.flaws.is_empty() {
            self.last_flaws = checked.flaws.clone();
            let (changed, kept) = unchanged();
            return Ok(Declared { accepted: false, changed, kept, revision: self.revision, added, replaced, flaws: checked.flaws, warnings: checked.warnings });
        }
        // What this declaration changed in the image: a definition whose own text or whose
        // dependencies differ is new state; every other keeps what was derived for it.
        self.ensure_current();
        let before = Image::build(self.current.as_ref().expect("current"));
        let after = Image::build(&checked);
        let mut changed = Vec::new();
        let mut kept = Vec::new();
        for (i, def) in after.defs.iter().enumerate() {
            let old = before.defs.iter().position(|d| d.key == def.key);
            if old.is_some_and(|o| before.closure[o] == after.closure[i]) {
                kept.push(def.key.clone());
            } else {
                changed.push(def.key.clone());
            }
        }
        for def in &before.defs {
            if !after.defs.iter().any(|d| d.key == def.key) {
                changed.push(def.key.clone());
            }
        }
        self.decls = next;
        self.last_flaws.clear();
        self.revision += 1;
        let warnings = checked.warnings.clone();
        self.current = Some(checked);
        Ok(Declared { accepted: true, changed, kept, revision: self.revision, added, replaced, flaws: Vec::new(), warnings })
    }

    /// Remove a declaration by key or by name (all with that name).
    pub fn remove(&mut self, name: &str) -> Vec<String> {
        self.current = None;
        self.revision += 1;
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.decls).into_iter().partition(|(key, _)| {
            key != name && !key.split_whitespace().nth(1).is_some_and(|n| n.split('/').next() == Some(name))
        });
        self.decls = keep;
        gone.into_iter().map(|(key, _)| key).collect()
    }

    pub fn load_text(&mut self, text: &str) -> Result<Declared, Vec<Flaw>> {
        self.declare(text)
    }

    pub fn scope(&self) -> Vec<ScopeEntry> {
        let checked = self.check();
        let Some(module) = &checked.module else {
            return checked
                .decls
                .iter()
                .map(|d| ScopeEntry { kind: d.kind, name: d.name.clone(), signature: d.key.clone() })
                .collect();
        };
        scope_of(module)
    }

    fn base_text(&self, target: &Target) -> Option<String> {
        match target {
            Target::Session => Some(self.source()),
            Target::Text(text) => Some(text.clone()),
            Target::Package(_) => None,
        }
    }

    pub fn eval(&mut self, target: &Target, what: &EvalWhat) -> EvalOutcome {
        self.counter += 1;
        if let Target::Package(path) = target {
            return eval_package(path, what);
        }
        let base = self.base_text(target).expect("text target");
        let checked = check_text(SESSION_FILE, &base);
        let Some(module) = checked.module else {
            return EvalOutcome::Flawed(checked.flaws);
        };
        match what {
            EvalWhat::Expression(expression) => eval_expression(&base, expression),
            EvalWhat::Call { function, arguments } => {
                let ty = declared_result(&module, function);
                typed(run(vec![module], |i| call(i, function, arguments), arguments), ty)
            }
            EvalWhat::Entry => {
                let ty = module.checked().entry.map(|e| show_type(module.checked(), module.checked().functions[e as usize].result, &[])).unwrap_or_default();
                typed(run(vec![module], |i| i.run_entry(), &[]), ty)
            }
        }
    }

    /// The type of an expression, and whether it can be known now.
    pub fn type_of(&self, expression: &str) -> Result<String, Vec<Flaw>> {
        match synthesize(&self.source(), expression) {
            Ok((module, _label)) => Ok(result_type(&module)),
            Err(flaws) => Err(flaws),
        }
    }

    /// The holes of the accumulated module.
    pub fn holes(&self) -> Checked {
        self.check()
    }
}

/// What `inspect` derives for `name` from the checked session: the shape and layout, and for a
/// function the value. This is the state that is kept per definition and saved in snapshots.
pub(crate) fn derive(checked: &Checked, name: &str) -> Json {
    let head = name.split('(').next().unwrap_or(name).trim();
    let mut fresh = crate::inspect::inspect(checked, name);
    if fresh["kind"] == "function" {
        if let Some(value) = crate::inspect::value_of_function(checked, head) {
            fresh["value"] = value;
        }
    }
    fresh
}

pub fn message_flaw(message: &str) -> Flaw {
    let text: std::sync::Arc<str> = std::sync::Arc::from("");
    let d = diagnostic::Diagnostic::new(diagnostic::code::SourceCheckCode::Malformed, message);
    crate::flaw::from_diagnostic("parse", &d, "<submission>", &text, &[])
}

fn scope_of(module: &CheckedSourceModule) -> Vec<ScopeEntry> {
    let file = module.checked();
    let mut out = Vec::new();
    for (index, f) in file.functions.iter().enumerate() {
        if module.function_home(index).is_some_and(|(home, own)| home == module.name() && own as usize == index) && !f.name.starts_with("__gin_") {
            let params = f.parameters.iter().map(|t| show_type(file, *t, &f.type_parameters)).collect::<Vec<_>>().join(", ");
            let types = if f.type_parameters.is_empty() { String::new() } else { format!("[{}]", f.type_parameters.join(", ")) };
            out.push(ScopeEntry { kind: "function", name: f.name.clone(), signature: format!("{}{types}({params}) -> {}", f.name, show_type(file, f.result, &f.type_parameters)) });
        }
    }
    for a in file.aggregates.iter().filter(|a| !a.name.contains('.')) {
        out.push(ScopeEntry { kind: "type", name: a.name.clone(), signature: format!("{:?}", a.kind).to_lowercase() });
    }
    out
}

/// Check `base` plus a function returning `expression`, trying result types until one fits.
fn synthesize(base: &str, expression: &str) -> Result<(CheckedSourceModule, String), Vec<Flaw>> {
    synthesize_inner(base, expression).map_err(|mut flaws| {
        // The wrapper function is ours: a flaw inside it is the expression's, not a place in
        // the session's source.
        for flaw in &mut flaws {
            if flaw.span.as_ref().is_some_and(|s| s.byte_start >= base.len()) {
                flaw.span = None;
                flaw.construct = None;
                flaw.file = "<expression>".to_owned();
                flaw.message = format!("in `{expression}`: {}", flaw.message);
            }
        }
        flaws
    })
}

fn synthesize_inner(base: &str, expression: &str) -> Result<(CheckedSourceModule, String), Vec<Flaw>> {
    let body: String = expression.lines().collect::<Vec<_>>().join("\n        ");
    let attempt = |annotation: &str| {
        let text = format!("{base}\n__gin_eval() {annotation}:\n    return {body}\n");
        check_once(&text)
    };
    let first = attempt(WIDE);
    let first_flaws = match first {
        Ok(module) => return Ok((module, "integer".to_owned())),
        Err(flaws) => flaws,
    };
    // Only a type mismatch says the result is not an integer; any other flaw is the answer.
    if first_flaws.iter().any(|f| f.code != "source-check-type") {
        return Err(first_flaws);
    }
    let names: Vec<String> = match check_once(base) {
        Ok(module) => module.checked().aggregates.iter().filter(|a| a.parameters.is_empty() && !a.name.contains('.')).map(|a| a.name.clone()).collect(),
        Err(flaws) => return Err(flaws),
    };
    for name in names {
        if let Ok(module) = attempt(&name) {
            return Ok((module, name));
        }
    }
    if let Ok(module) = check_once(&format!("{base}\n__gin_eval():\n    return {body}\n")) {
        return Ok((module, "()".to_owned()));
    }
    Err(first_flaws)
}

pub fn check_once(text: &str) -> Result<CheckedSourceModule, Vec<Flaw>> {
    let source: std::sync::Arc<str> = std::sync::Arc::from(text);
    let parsed = parser::parse_strict(text).map_err(|ds| {
        ds.iter().map(|d| crate::flaw::from_diagnostic("parse", d, SESSION_FILE, &source, &[])).collect::<Vec<_>>()
    })?;
    let decls = crate::source::declarations(&parsed);
    typecheck::check_module_diagnostic("main", &parsed, &[], typecheck::Limits::default())
        .map_err(|failure| vec![crate::check::check_failure_flaw(&failure, SESSION_FILE, &source, &decls)])
}

fn result_type(module: &CheckedSourceModule) -> String {
    let file = module.checked();
    let Some(index) = file.functions.iter().position(|f| f.name == "__gin_eval") else { return "?".to_owned() };
    if let Some(body) = &file.bodies[index] {
        if let Some(CheckedSourceBodyKind::Block { result, .. }) = body.expressions.get(body.root as usize).map(|e| &e.kind) {
            if let Some(CheckedSourceBodyKind::Return { value }) = body.expressions.get(*result as usize).map(|e| &e.kind) {
                return show_type(file, body.expressions[*value as usize].ty, &[]);
            }
        }
    }
    show_type(file, file.functions[index].result, &[])
}

fn eval_expression(base: &str, expression: &str) -> EvalOutcome {
    match synthesize(base, expression) {
        Err(flaws) => EvalOutcome::Flawed(flaws),
        Ok((module, label)) => {
            let ty = if label == "integer" { result_type(&module) } else { label };
            let mut outcome = run(vec![module], |i| i.call("__gin_eval", &[], &[]), &[]);
            if let EvalOutcome::Value { ty: slot, .. } = &mut outcome {
                *slot = ty;
            }
            outcome
        }
    }
}

fn call(interpreter: &mut Interpreter<'_>, function: &str, arguments: &[Arg]) -> Result<interpreter::Outcome, Error> {
    let values: Vec<Value> = arguments
        .iter()
        .filter_map(|a| match a {
            Arg::Known(v) => Some(v.clone()),
            Arg::Unknown => None,
        })
        .collect();
    interpreter.call(function, &[], &values)
}

fn run(
    modules: Vec<CheckedSourceModule>,
    go: impl FnOnce(&mut Interpreter<'_>) -> Result<interpreter::Outcome, Error>,
    arguments: &[Arg],
) -> EvalOutcome {
    if let Some(position) = arguments.iter().position(|a| matches!(a, Arg::Unknown)) {
        return unknown(json!({
            "kind": "unknown_input",
            "detail": format!("argument {position} is not known at compile time"),
        }));
    }
    let program = Program::from_modules(modules);
    finish(&program, go(&mut Interpreter::new(&program)))
}

fn eval_package(path: &Path, what: &EvalWhat) -> EvalOutcome {
    let checked = check_package(path);
    if !checked.flaws.is_empty() {
        return EvalOutcome::Flawed(checked.flaws);
    }
    let program = match Program::from_package(path) {
        Ok(program) => program,
        Err(error) => return unknown(json!({"kind": "load_failed", "detail": error.to_string()})),
    };
    let mut interpreter = Interpreter::new(&program);
    let outcome = match what {
        EvalWhat::Entry => interpreter.run_entry(),
        EvalWhat::Call { function, arguments } => {
            if arguments.iter().any(|a| matches!(a, Arg::Unknown)) {
                return unknown(json!({"kind": "unknown_input", "detail": "an argument is not known at compile time"}));
            }
            call(&mut interpreter, function, arguments)
        }
        EvalWhat::Expression(_) => {
            return unknown(json!({"kind": "unsupported_target", "detail": "an expression is evaluated against the session or a source text; for a package give a call or the entry"}));
        }
    };
    finish(&program, outcome)
}

fn declared_result(module: &CheckedSourceModule, function: &str) -> String {
    let file = module.checked();
    file.functions
        .iter()
        .find(|f| f.name == function)
        .map(|f| show_type(file, f.result, &f.type_parameters))
        .unwrap_or_default()
}

fn typed(mut outcome: EvalOutcome, ty: String) -> EvalOutcome {
    if let EvalOutcome::Value { ty: slot, .. } = &mut outcome {
        *slot = ty;
    }
    outcome
}

fn unknown(reason: Json) -> EvalOutcome {
    let text = reason["detail"].as_str().unwrap_or("").to_owned();
    EvalOutcome::Unknown { reason, text }
}

fn finish(program: &Program, outcome: Result<interpreter::Outcome, Error>) -> EvalOutcome {
    match outcome {
        Ok(outcome) => EvalOutcome::Value {
            json: value_json(program, &outcome.value),
            display: program.render(&outcome.value),
            ty: String::new(),
        },
        Err(error) => unknown(reason_of(&error)),
    }
}

pub fn reason_of(error: &Error) -> Json {
    let detail = error.to_string();
    match error {
        Error::NoBody(function) => json!({"kind": "unrecorded_body", "function": function, "detail": detail}),
        Error::Unsupported { what, function } if what.contains("instruction") => {
            json!({"kind": "instruction_refused", "function": function, "detail": detail})
        }
        Error::Unsupported { function, .. } => json!({"kind": "unsupported", "function": function, "detail": detail}),
        Error::HoleReached { function, stack, .. } => {
            json!({"kind": "hole_reached", "function": function, "call_chain": stack, "detail": detail})
        }
        Error::ArgumentOutOfDomain { function, .. } => json!({"kind": "argument_out_of_domain", "function": function, "detail": detail}),
        Error::DomainViolated { function, .. } => json!({"kind": "checker_bug", "function": function, "detail": detail}),
        Error::OutOfFuel(_) | Error::DepthExceeded(_) => json!({"kind": "out_of_fuel", "detail": detail}),
        Error::NoSuchFunction(_) | Error::Arity { .. } | Error::TypeArity { .. } => json!({"kind": "invalid_call", "detail": detail}),
        _ => json!({"kind": "internal", "detail": detail}),
    }
}

pub fn value_json(program: &Program, value: &Value) -> Json {
    match value {
        Value::Unit => json!({"kind": "unit"}),
        Value::Integer(n) => match i64::try_from(n.as_i128()) {
            Ok(small) if n.as_i128() == i128::from(small) => json!({"kind": "integer", "value": small}),
            _ => json!({"kind": "integer", "value": n.to_string()}),
        },
        Value::Text(text) => json!({"kind": "text", "value": text}),
        Value::Record { tag, fields } => {
            let names = program.field_names(tag);
            let fields: Vec<Json> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| json!({"name": names.get(i).filter(|_| names.len() == fields.len()), "value": value_json(program, f)}))
                .collect();
            json!({"kind": "record", "type": tag.short(), "sum": tag.sum(), "home": &*tag.home, "fields": fields})
        }
    }
}

/// Parse a JSON request argument into a value; `null` or `{"unknown": ...}` is unknown.
pub fn arg_of(json: &Json) -> Result<Arg, String> {
    match json {
        Json::Null => Ok(Arg::Unknown),
        Json::Object(o) if o.contains_key("unknown") => Ok(Arg::Unknown),
        Json::Number(n) => n.as_i64().map(|v| Arg::Known(Value::integer(i128::from(v)))).ok_or_else(|| "integers must fit 64 bits; pass larger ones as strings".to_owned()),
        Json::String(s) => s.parse::<i128>().map(|v| Arg::Known(Value::integer(v))).map_err(|_| format!("`{s}` is not an integer")),
        other => Err(format!("unsupported argument {other}; integers (and null for unknown) only")),
    }
}
