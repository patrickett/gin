//! Checking a source text or a package into structured flaws, warnings and holes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use diagnostic::Diagnostic;
use serde_json::{Value, json};
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceFile, CheckedSourceModule, Limits, SourceCheckFailure,
    SourceCheckNote, SourceCheckNoteKind, SourceFact, SourceType, check_module_diagnostic,
};

use crate::flaw::{Flaw, Located, construct_at, fact_json, fact_text, from_diagnostic, param_json};
use crate::source::{Decl, ParamInfo, declarations};

pub const SESSION_FILE: &str = "<session>";
const REMOVALS_MAX: usize = 32;

/// The result of checking a text: every flaw that could be found, and the checked module when
/// nothing was wrong.
pub struct Checked {
    pub file: String,
    pub source: Arc<str>,
    pub decls: Vec<Decl>,
    pub flaws: Vec<Flaw>,
    pub warnings: Vec<Flaw>,
    /// Present only when the whole text checked.
    pub module: Option<CheckedSourceModule>,
    /// Holes found in what did check (all of it, when `module` is present).
    pub holes: Vec<HoleInfo>,
}

impl Checked {
    pub fn complete(&self) -> bool {
        self.module.is_some()
    }

    pub fn all(&self) -> impl Iterator<Item = &Flaw> {
        self.flaws.iter().chain(&self.warnings)
    }
}

#[derive(Debug, Clone)]
pub struct HoleInfo {
    pub span: Located,
    pub construct: String,
    pub expected: Option<String>,
    pub expected_reason: Option<&'static str>,
    pub params: Vec<ParamInfo>,
    pub locals: Vec<(String, Option<String>)>,
    pub functions: Vec<String>,
    pub types: Vec<String>,
    /// The checker's facts where the hole stands, as claims, and structured.
    pub facts: Vec<String>,
    pub proved: Vec<SourceFact>,
}

impl HoleInfo {
    pub fn json(&self) -> Value {
        json!({
            "span": self.span.json(),
            "construct": self.construct,
            "expected_type": self.expected,
            "expected_type_reason": self.expected_reason,
            "in_scope": {
                "parameters": self.params.iter().map(param_json).collect::<Vec<_>>(),
                "locals": self.locals.iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>(),
                "functions": self.functions,
                "types": self.types,
            },
            // What the checker proved where the hole stands, guards on the path included.
            "facts": self.proved.iter().map(fact_json).collect::<Vec<_>>(),
            "fact_claims": self.facts,
        })
    }
}

fn limits() -> Limits {
    let defaults = Limits::default();
    Limits { work_max: defaults.work_max * 10, ..defaults }
}

/// Blank `start..end` with spaces, keeping every newline, so later spans keep their offsets.
pub fn blank_source_range(text: &str, start: usize, end: usize) -> String {
    assert!(start <= end);
    assert!(end <= text.len());
    assert!(text.is_char_boundary(start));
    assert!(text.is_char_boundary(end));
    let mut out = text.as_bytes().to_vec();
    for byte in &mut out[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
    let out = String::from_utf8(out).expect("replacing UTF-8 bytes with ASCII preserves UTF-8");
    assert_eq!(out.len(), text.len());
    assert_eq!(out.bytes().filter(|b| *b == b'\n').count(), text.bytes().filter(|b| *b == b'\n').count());
    out
}

pub fn check_failure_flaw(
    failure: &SourceCheckFailure,
    file: &str,
    source: &Arc<str>,
    decls: &[Decl],
) -> Flaw {
    // The diagnostic itself is `ginc`'s, the one `--check-new` prints. The JSON keeps the
    // enclosing construct as `span` and the exact expression as `culprit`.
    let shared = ginc::failure_diagnostic(failure);
    let (_, general) = ginc::failure_details(failure.error);
    let mut by_construct = Diagnostic::new(shared.code, shared.message.as_str());
    if let Some(span) = failure.span {
        by_construct = by_construct.at_span(span);
    }
    let mut flaw = from_diagnostic("check", &by_construct, file, source, decls);
    flaw.claim = Some(general.to_owned());
    flaw.culprit = failure.culprit.map(|culprit| Located::new(file, source, culprit));
    flaw.proved = (!failure.facts.is_empty()).then(|| failure.facts.clone());
    flaw.diagnostic = shared;
    flaw
}

fn note_flaw(note: &SourceCheckNote, file: &str, source: &Arc<str>, decls: &[Decl]) -> Flaw {
    let shared = ginc::note_diagnostic(note);
    let mut flaw = from_diagnostic("check", &shared, file, source, decls);
    flaw.proved = (!note.facts.is_empty()).then(|| note.facts.clone());
    flaw.diagnostic = shared;
    flaw
}

/// Check one text. After a flaw, the declaration it is in is set aside and the rest checked
/// again, so one run reports every independent flaw (the checker itself stops at the first).
/// Flaws found after a removal say which declarations were set aside: they may be knock-ons.
pub fn check_text(file: &str, text: &str) -> Checked {
    let source: Arc<str> = Arc::from(text);
    let parsed = match parser::parse_strict(text) {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            let flaws = diagnostics.iter().map(|d| from_diagnostic("parse", d, file, &source, &[])).collect();
            return Checked { file: file.to_owned(), source, decls: Vec::new(), flaws, warnings: Vec::new(), module: None, holes: Vec::new() };
        }
    };
    let decls = declarations(&parsed);
    let mut working = text.to_owned();
    let mut removed: Vec<String> = Vec::new();
    let mut flaws = Vec::new();
    let mut warnings = Vec::new();
    let mut module = None;
    let mut holes = Vec::new();
    let mut last_removed: Option<usize> = None;
    for _ in 0..=REMOVALS_MAX {
        let Ok(current) = parser::parse_strict(&working) else { break };
        match check_module_diagnostic("main", &current, &[], limits()) {
            Ok(checked) => {
                warnings = checked.checked().notes.iter().map(|n| note_flaw(n, file, &source, &decls)).collect();
                holes = holes_of(&checked, &[(file.to_owned(), Arc::clone(&source))]);
                if removed.is_empty() {
                    module = Some(checked);
                }
                break;
            }
            Err(failure) => {
                let mut flaw = check_failure_flaw(&failure, file, &source, &decls);
                flaw.knock_on_of = removed.clone();
                let culprit = failure.span.and_then(|span| {
                    decls.iter().position(|d| d.start <= span.start() && span.end() <= d.end)
                });
                flaws.push(flaw);
                let Some(index) = culprit.filter(|i| last_removed != Some(*i)) else { break };
                last_removed = Some(index);
                let d = &decls[index];
                removed.push(d.name.clone());
                working = blank_source_range(&working, d.start, d.end);
            }
        }
    }
    flaws.sort_by_key(Flaw::sort_key);
    warnings.sort_by_key(Flaw::sort_key);
    Checked { file: file.to_owned(), source, decls, flaws, warnings, module, holes }
}

/// Check a package directory. The checker stops at the first failing module; a package reports
/// that one flaw, and the warnings and holes of every module when all of them check.
pub fn check_package(path: &Path) -> Checked {
    let name = path.display().to_string();
    match package::Package::open(path) {
        Ok(package) => check_opened_package(&name, &package, &|p| std::fs::read_to_string(p).ok()).0,
        Err(error) => check_package_unopened(&name, error.to_string()),
    }
}

fn check_package_unopened(name: &str, message: String) -> Checked {
    let empty: Arc<str> = Arc::from("");
    let diagnostic = Diagnostic::new(diagnostic::code::SourceCheckCode::Malformed, message.as_str());
    let mut flaw = from_diagnostic("check", &diagnostic, name, &empty, &[]);
    flaw.code = "package-load".to_owned();
    Checked { file: name.to_owned(), source: empty, decls: Vec::new(), flaws: vec![flaw], warnings: Vec::new(), module: None, holes: Vec::new() }
}

/// Check an opened package, reading failing files back through `read` (an editor passes its
/// unsaved buffers; the package itself must read the same ones, see
/// `package::Package::with_overlay`). Also returns the files the package loaded, empty when
/// loading itself failed.
pub fn check_opened_package(
    name: &str,
    package: &package::Package,
    read: &dyn Fn(&Path) -> Option<String>,
) -> (Checked, Vec<PathBuf>) {
    let name = name.to_owned();
    let empty: Arc<str> = Arc::from("");
    let unlocated = |code: &str, message: String| {
        let diagnostic = Diagnostic::new(diagnostic::code::SourceCheckCode::Malformed, message.as_str());
        let mut flaw = from_diagnostic("check", &diagnostic, &name, &empty, &[]);
        flaw.code = code.to_owned();
        flaw
    };
    let fail = |flaws| (Checked { file: name.clone(), source: Arc::clone(&empty), decls: Vec::new(), flaws, warnings: Vec::new(), module: None, holes: Vec::new() }, Vec::new());
    match package.check(package::GraphLimits::default(), limits()) {
        Ok(checked) => {
            let mut warnings = Vec::new();
            let mut holes = Vec::new();
            for (index, capsule) in checked.capsules.iter().enumerate() {
                let module = &checked.graph.modules[index];
                let files: Vec<(String, Arc<str>)> = module
                    .paths
                    .iter()
                    .zip(&module.files)
                    .map(|(p, f)| (p.display().to_string(), Arc::clone(&f.text)))
                    .collect();
                for note in &capsule.checked().notes {
                    let (file, source) = note.file.and_then(|i| files.get(i)).cloned().unwrap_or((name.clone(), Arc::clone(&empty)));
                    let decls = module.files.iter().position(|f| Arc::ptr_eq(&f.text, &source)).map(|i| declarations(&module.files[i])).unwrap_or_default();
                    warnings.push(note_flaw(note, &file, &source, &decls));
                }
                holes.extend(holes_of(capsule, &files));
            }
            warnings.sort_by_key(Flaw::sort_key);
            let loaded = checked.graph.modules.iter().flat_map(|m| m.paths.iter().cloned()).collect();
            (Checked { file: name, source: empty, decls: Vec::new(), flaws: Vec::new(), warnings, module: None, holes }, loaded)
        }
        Err(package::PackageError::Check { module, path: file, failure }) => {
            let flaw = match file.and_then(|p| Some((p.display().to_string(), read(&p)?))) {
                Some((file, text)) => {
                    let source: Arc<str> = Arc::from(text.as_str());
                    let decls = parser::parse_strict(&text).map(|f| declarations(&f)).unwrap_or_default();
                    check_failure_flaw(&failure, &file, &source, &decls)
                }
                None => {
                    let mut flaw = check_failure_flaw(&failure, &name, &empty, &[]);
                    flaw.message = format!("in module `{module}`: {}", flaw.message);
                    flaw
                }
            };
            fail(vec![flaw])
        }
        Err(package::PackageError::Graph(package::GraphError::Load(package::LoadError::Syntax { path, diagnostics }))) => {
            let text = read(&path).unwrap_or_default();
            let source: Arc<str> = Arc::from(text.as_str());
            let file = path.display().to_string();
            fail(diagnostics.iter().map(|d| from_diagnostic("parse", d, &file, &source, &[])).collect())
        }
        Err(error) => fail(vec![unlocated("package-load", error.to_string())]),
    }
}

/// The path to show for a package argument that might be a file.
pub fn is_package(path: &Path) -> bool {
    path.is_dir()
}

pub fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

pub fn path_of(text: &str) -> PathBuf {
    PathBuf::from(text)
}

// ---------------------------------------------------------------------------------------
// types and holes

/// A type as source would write it, as far as the checked file knows it.
pub fn show_type(file: &CheckedSourceFile, ty: SourceType, type_parameters: &[String]) -> String {
    let all = |ids: &[SourceType]| ids.iter().map(|t| show_type(file, *t, type_parameters)).collect::<Vec<_>>().join(", ");
    match ty {
        SourceType::Unit => "()".to_owned(),
        SourceType::Text => "text".to_owned(),
        SourceType::Integer { lower, upper } => format!("{lower} to {upper}"),
        SourceType::Generic { index } => type_parameters.get(index as usize).cloned().unwrap_or_else(|| format!("type parameter {index}")),
        SourceType::Aggregate { id } | SourceType::Sum { id } | SourceType::Array { id } | SourceType::Sequence { id } | SourceType::Pointer { id } => {
            use typecheck::CheckedSourceInstanceHead as Head;
            let Some(instance) = file.instances.get(id as usize) else { return "?".to_owned() };
            match instance.head {
                Head::Nominal { schema } => {
                    let name = file.aggregates.get(schema as usize).map_or("?", |a| a.name.as_str());
                    if instance.arguments.is_empty() { name.to_owned() } else { format!("{name}({})", all(&instance.arguments)) }
                }
                Head::Array { extent } => format!("[{}; {extent}]", all(&instance.arguments)),
                Head::Slice | Head::Sized => format!("[{}; n]", all(&instance.arguments[..1.min(instance.arguments.len())])),
                Head::Sequence => format!("seq({})", all(&instance.arguments)),
                Head::Pointer => format!("ptr({})", all(&instance.arguments)),
            }
        }
    }
}

fn signature(file: &CheckedSourceFile, index: usize) -> String {
    let f = &file.functions[index];
    let params = f.parameters.iter().map(|t| show_type(file, *t, &f.type_parameters)).collect::<Vec<_>>().join(", ");
    format!("{}({params}) -> {}", f.name, show_type(file, f.result, &f.type_parameters))
}

/// The holes of `module`, with what is known about each. `files[i]` is file `i` of the module.
fn holes_of(module: &CheckedSourceModule, files: &[(String, Arc<str>)]) -> Vec<HoleInfo> {
    let checked = module.checked();
    let notes: Vec<&SourceCheckNote> = checked.notes.iter().filter(|n| n.kind == SourceCheckNoteKind::Hole).collect();
    if notes.is_empty() {
        return Vec::new();
    }
    let parsed: Vec<Vec<Decl>> = files
        .iter()
        .map(|(_, text)| parser::parse_strict(text).map(|f| declarations(&f)).unwrap_or_default())
        .collect();
    let own: Vec<usize> = (0..checked.functions.len())
        .filter(|i| module.function_home(*i).is_some_and(|(home, idx)| home == module.name() && idx as usize == *i))
        .collect();
    let functions: Vec<String> = own.iter().map(|i| signature(checked, *i)).collect();
    let types: Vec<String> = checked.aggregates.iter().filter(|a| !a.name.contains('.')).map(|a| a.name.clone()).collect();
    // Holes already paired with a recorded `Hole` node, per construct.
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    let mut out = Vec::new();
    for note in notes {
        let (Some(span), Some(file_index)) = (note.span, note.file) else { continue };
        let Some((file, source)) = files.get(file_index) else { continue };
        let decls = &parsed[file_index];
        let Some(construct) = construct_at(file, source, decls, span) else { continue };
        let located = Located::new(file, source, span);
        let (params, locals) = decls
            .iter()
            .flat_map(|d| std::iter::once((d.name.clone(), d.start, d.end, &d.params, &d.locals)).chain(d.members.iter().map(|m| (m.name.clone(), m.start, m.end, &m.params, &m.locals))))
            .find(|(name, ..)| *name == construct.name)
            .map(|(_, _, _, params, locals)| (params.clone(), locals.iter().filter(|(end, _)| *end <= span.start()).map(|(_, l)| (l.name.clone(), l.ty.clone())).collect::<Vec<_>>()))
            .unwrap_or_default();
        let position = {
            let slot = seen.entry(construct.name.clone()).or_insert(0);
            *slot += 1;
            *slot - 1
        };
        let function = own.iter().copied().find(|i| checked.functions[*i].name == construct.name);
        let nodes: Vec<SourceType> = function
            .and_then(|i| checked.bodies[i].as_ref())
            .map(|b| b.expressions.iter().filter(|e| matches!(e.kind, CheckedSourceBodyKind::Hole)).map(|e| e.ty).collect())
            .unwrap_or_default();
        let total = checked.notes.iter().filter(|n| n.kind == SourceCheckNoteKind::Hole && n.span.is_some_and(|s| construct_at(file, source, decls, s).is_some_and(|c| c.name == construct.name))).count();
        let (expected, expected_reason) = match (function, nodes.get(position)) {
            (Some(i), Some(ty)) if nodes.len() == total => (Some(show_type(checked, *ty, &checked.functions[i].type_parameters)), None),
            _ => (None, Some("the recorder does not place this hole, so its type is not published")),
        };
        let facts: Vec<String> = note.facts.iter().map(fact_text).collect();
        out.push(HoleInfo { span: located, construct: construct.name, expected, expected_reason, params, locals, functions: functions.clone(), types: types.clone(), facts, proved: note.facts.clone() });
    }
    out.sort_by(|a, b| (a.span.file.as_str(), a.span.byte_start).cmp(&(b.span.file.as_str(), b.span.byte_start)));
    out
}
