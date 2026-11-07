//! Hover: what `$inspect` says about the name under the cursor, as markdown.
//!
//! `inspect` needs a module that checked, so the document is inspected with its flawed
//! declarations blanked (they are reported as diagnostics instead). A name that is not a
//! top-level type, alias, constant or function falls back to what was written: a parameter or
//! a local with its annotation. All numbers (size, alignment, offsets, registers, values) are
//! the checker's and lowering's, through `ginterpreter::inspect`.

use ginterpreter::check::{Checked, HoleInfo, check_text};
use ginterpreter::inspect::{inspect, value_of_function};
use ginterpreter::source::{Decl, declarations};
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};
use serde_json::Value as Json;
use ast::SourceFile;

use crate::analysis::without_flawed;
use crate::position::LineIndex;

/// Everything hover needs about one version of one document, built once.
pub struct HoverState {
    /// The check of the document as written (its flaws and holes).
    pub original: Checked,
    /// The check with flawed declarations blanked, when the document has flaws; `module` is
    /// present when that checks.
    blanked: Option<Checked>,
    /// The recovered syntax tree, which exists even when the text has syntax errors.
    pub syntax: Option<SourceFile>,
    pub decls: Vec<Decl>,
}

impl HoverState {
    pub fn new(file: &str, text: &str) -> Self {
        let original = check_text(file, text);
        let syntax = parser::parse(text).ok().map(|output| output.file);
        let decls = syntax.as_ref().map(declarations).unwrap_or_default();
        let blanked = (original.module.is_none() && !original.decls.is_empty()).then(|| without_flawed(file, text, &original));
        Self { original, blanked, syntax, decls }
    }

    /// The check `inspect` runs against: the document, or the document without its flaws.
    pub fn inspectable(&self) -> &Checked {
        self.blanked.as_ref().unwrap_or(&self.original)
    }
}

/// The identifier containing byte `offset` (or ending at it).
pub fn word_at(text: &str, offset: usize) -> Option<(usize, usize)> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let offset = offset.min(text.len());
    if !text.is_char_boundary(offset) {
        return None;
    }
    let start = text[..offset].char_indices().rev().take_while(|(_, c)| is_word(*c)).last().map_or(offset, |(i, _)| i);
    let end = offset + text[offset..].char_indices().take_while(|(_, c)| is_word(*c)).last().map_or(0, |(i, c)| i + c.len_utf8());
    let word = &text[start..end];
    (!word.is_empty() && !word.starts_with(|c: char| c.is_ascii_digit())).then_some((start, end))
}

fn in_comment(syntax: &SourceFile, offset: usize) -> bool {
    syntax.comments.iter().any(|c| {
        let span = syntax.span_table.get(c.span_id);
        span.start() <= offset && offset < span.end()
    })
}

pub fn hover(state: &HoverState, text: &str, index: &LineIndex, offset: usize) -> Option<Hover> {
    if state.syntax.as_ref().is_some_and(|s| in_comment(s, offset)) {
        return None;
    }
    // A `#todo` hole: the type it stands for and what is known there.
    if let Some(hole) = state.original.holes.iter().find(|h| h.span.byte_start <= offset && offset <= h.span.byte_end) {
        return Some(markup(hole_markdown(hole), index.range(text, hole.span.byte_start, hole.span.byte_end)));
    }
    let (start, end) = word_at(text, offset)?;
    let word = &text[start..end];
    let range = index.range(text, start, end);
    if state.inspectable().module.is_some() {
        let mut json = inspect(state.inspectable(), word);
        if json["found"] == true {
            if json["kind"] == "function" {
                if let Some(value) = value_of_function(state.inspectable(), word) {
                    json["value"] = value;
                }
            }
            let mut body = inspect_markdown(&json);
            if let Some(flaw) = flaw_in_declaration(state, offset) {
                body.push_str(&format!("\n\n---\nThis declaration has a flaw (`{flaw}`)."));
            }
            return Some(markup(body, range));
        }
    }
    written(state, word, offset).map(|body| markup(body, range))
}

fn flaw_in_declaration(state: &HoverState, offset: usize) -> Option<String> {
    state.original.flaws.iter().find_map(|f| {
        let span = f.span.as_ref()?;
        (span.byte_start <= offset && offset <= span.byte_end).then(|| f.code.clone())
    })
}

fn markup(value: String, range: lsp_types::Range) -> Hover {
    Hover { contents: HoverContents::Markup(MarkupContent { kind: MarkupKind::Markdown, value }), range: Some(range) }
}

/// A parameter, a local before `offset`, or a top-level declaration, as written.
fn written(state: &HoverState, word: &str, offset: usize) -> Option<String> {
    let enclosing = state
        .decls
        .iter()
        .flat_map(|d| d.members.iter().map(|m| (m.start, m.end, &m.params, &m.locals)).chain(std::iter::once((d.start, d.end, &d.params, &d.locals))))
        .filter(|(start, end, ..)| *start <= offset && offset <= *end)
        .min_by_key(|(start, end, ..)| end - start);
    if let Some((_, _, params, locals)) = enclosing {
        if let Some(p) = params.iter().find(|p| p.name == word) {
            let convention = p.convention.as_deref().map(|c| format!("{c} ")).unwrap_or_default();
            return Some(match &p.ty {
                Some(ty) => format!("```gin\n{convention}{} {ty}\n```\nparameter", p.name),
                None => format!("```gin\n{convention}{}\n```\nparameter (type inferred)", p.name),
            });
        }
        if let Some((_, l)) = locals.iter().rev().find(|(end, l)| l.name == word && *end <= offset) {
            return Some(match &l.ty {
                Some(ty) => format!("```gin\n{} {ty}\n```\nlocal", l.name),
                None => format!("```gin\n{}\n```\nlocal", l.name),
            });
        }
    }
    let decl = state.decls.iter().find(|d| d.name == word)?;
    let mut signature = decl.name.clone();
    if decl.kind == "function" {
        let params: Vec<String> = decl
            .params
            .iter()
            .map(|p| match (&p.convention, &p.ty) {
                (Some(c), Some(t)) => format!("{c} {} {t}", p.name),
                (None, Some(t)) => format!("{} {t}", p.name),
                _ => p.name.clone(),
            })
            .collect();
        signature = format!("{}({})", decl.name, params.join(", "));
        if let Some(result) = &decl.result {
            signature.push(' ');
            signature.push_str(result);
        }
    }
    let note = if state.original.flaws.is_empty() {
        ""
    } else {
        "\n\nThe checker has no detail for this declaration while the file has flaws."
    };
    Some(format!("```gin\n{signature}\n```\n{}{note}", decl.kind))
}

fn hole_markdown(hole: &HoleInfo) -> String {
    let mut out = String::from("**`#todo` hole**");
    match &hole.expected {
        Some(expected) => out.push_str(&format!(" in `{}`: expected `{expected}`", hole.construct)),
        None => out.push_str(&format!(" in `{}`: {}", hole.construct, hole.expected_reason.unwrap_or("expected type unknown"))),
    }
    if !hole.facts.is_empty() {
        out.push_str("\n\nKnown here:\n");
        for fact in &hole.facts {
            out.push_str(&format!("- `{fact}`\n"));
        }
    }
    out
}

fn available(value: &Json) -> String {
    if value["available"] == true {
        format!("{} {}", value["value"], value["unit"].as_str().unwrap_or(""))
    } else {
        format!("unavailable ({})", value["reason"]["detail"].as_str().unwrap_or("not computed"))
    }
}

fn str_of<'a>(value: &'a Json, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}

/// `inspect`'s JSON as markdown. Missing pieces are skipped, never guessed.
pub fn inspect_markdown(json: &Json) -> String {
    match json["kind"].as_str() {
        Some("type") => type_markdown(json),
        Some("function") => function_markdown(json),
        Some("constant") => format!(
            "```gin\n{} := {}\n```\nconstant of type `{}`",
            str_of(json, "name"),
            json["value"]["value"].as_str().map_or_else(|| json["value"]["value"].to_string(), str::to_owned),
            str_of(json, "type")
        ),
        Some("alias") => format!("```gin\n{} is {}\n```\nalias", str_of(json, "name"), str_of(json, "target")),
        other => format!("`{}` ({})", str_of(json, "name"), other.unwrap_or("unknown kind")),
    }
}

fn type_markdown(json: &Json) -> String {
    let mut out = format!("**{}**: {} type, {}", str_of(json, "name"), str_of(json, "type_kind"), str_of(json, "linearity"));
    if let Some(bounds) = json["domain"]["bounds"].as_object() {
        out.push_str(&format!("\n\ndomain `{} to {}`", bounds["lower"].as_str().unwrap_or("?"), bounds["upper"].as_str().unwrap_or("?")));
        if json["domain"]["wraps"] == true {
            out.push_str(", wraps");
        }
    }
    if let Some(variants) = json["variants"].as_array().filter(|v| !v.is_empty()) {
        out.push_str("\n\nVariants:\n");
        for v in variants {
            let payload: Vec<String> = v["payload"].as_array().into_iter().flatten().map(|p| format!("{}: {}", str_of(p, "name"), str_of(p, "type"))).collect();
            out.push_str(&format!("- `{}`{}\n", str_of(v, "name"), if payload.is_empty() { String::new() } else { format!(" ({})", payload.join(", ")) }));
        }
    } else if let Some(fields) = json["fields"].as_array().filter(|f| !f.is_empty() && json["type_kind"] != "domain") {
        out.push_str("\n\nFields:\n");
        let offsets = json["layout"]["field_offsets"]["fields"].as_array();
        for (i, f) in fields.iter().enumerate() {
            let at = offsets
                .and_then(|o| o.get(i))
                .map(|o| format!(" (offset {}, size {})", o["offset"], o["size"]))
                .unwrap_or_default();
            out.push_str(&format!("- `{}`: `{}`{at}\n", str_of(f, "name"), str_of(f, "type")));
        }
    }
    let layout = &json["layout"];
    out.push_str(&format!("\n\nSize {}; alignment {}", available(&layout["size"]), available(&layout["alignment"])));
    let registers = &layout["registers"];
    if registers["available"] == true {
        out.push_str(&format!("; {} of {} registers", registers["registers"], registers["limit"]));
    }
    if let Some(methods) = json["methods"].as_array().filter(|m| !m.is_empty()) {
        out.push_str("\n\nMethods:\n");
        for m in methods {
            out.push_str(&format!("- `{}`\n", m["signature"].as_str().unwrap_or_else(|| m["name"].as_str().unwrap_or("?"))));
        }
    }
    out
}

fn function_markdown(json: &Json) -> String {
    let mut out = String::new();
    for overload in json["overloads"].as_array().into_iter().flatten() {
        out.push_str(&format!("```gin\n{}\n```\n", str_of(overload, "signature")));
        let written = &overload["result"]["written"];
        if let Some(written) = written.as_str() {
            out.push_str(&format!("result `{written}`; "));
        }
        let body = &overload["body"];
        if body["recorded"] == true {
            out.push_str(&format!("body recorded ({} nodes{})\n", body["nodes"], if body["effects"] == true { ", has effects" } else { "" }));
        } else {
            out.push_str("body not recorded\n");
        }
    }
    let value = &json["value"];
    if value["known"] == true {
        out.push_str(&format!("\nEvaluates to `{}` : `{}`\n", str_of(value, "display"), str_of(value, "type")));
    }
    out
}
