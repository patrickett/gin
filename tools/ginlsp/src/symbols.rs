//! Queries over `ast` that need no checker: the outline of a file (document symbols),
//! names declared at the top level (go-to-definition, workspace symbols) and the names a file
//! imports. They work on the recovered tree, so they keep answering while the text has syntax
//! errors.

use ast::{Binding, Import, ImportSource, Item, SourceFile, TypeBody, TypeMember};

use lsp_types::{DocumentSymbol, SymbolKind};

use crate::position::LineIndex;

/// A top-level declared name and where its name is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub name: String,
    pub kind: SymbolKind,
    pub name_start: usize,
    pub name_end: usize,
    pub start: usize,
    pub end: usize,
}

fn binding_kind(binding: &Binding, in_type: bool) -> SymbolKind {
    match (binding.parameters.is_some(), in_type) {
        (true, true) => SymbolKind::METHOD,
        (true, false) => SymbolKind::FUNCTION,
        (false, true) => SymbolKind::FIELD,
        (false, false) => SymbolKind::CONSTANT,
    }
}

fn type_kind(body: &TypeBody) -> SymbolKind {
    match body {
        TypeBody::Has(_) => SymbolKind::STRUCT,
        TypeBody::Is(_) => SymbolKind::ENUM,
        TypeBody::Domain(_) => SymbolKind::NUMBER,
    }
}

/// The top-level bindings and types of `file`, in source order.
pub fn declared(file: &SourceFile) -> Vec<Declared> {
    file.items
        .iter()
        .filter_map(|item| {
            let span = file.span_table.get(item.span_id);
            let (name, kind) = match &item.value {
                Item::Binding(b) => (&b.name, binding_kind(b, false)),
                Item::Type(t) => (&t.name, type_kind(&t.body.value)),
                _ => return None,
            };
            let name_span = file.span_table.get(name.span_id);
            Some(Declared { name: name.value.clone(), kind, name_start: name_span.start(), name_end: name_span.end(), start: span.start(), end: span.end() })
        })
        .collect()
}

/// The dotted module path and selected names of each `use` entry: `use app.util five, Six`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub module: Vec<String>,
    /// (the name as exported, the name it is bound to here)
    pub members: Vec<(String, String)>,
}

pub fn imports(file: &SourceFile) -> Vec<Imported> {
    let mut out = Vec::new();
    for item in &file.items {
        let Item::Import(import) = &item.value else { continue };
        let Import { entries } = &**import;
        for entry in entries {
            let ImportSource::Module(path) = &entry.value.source.value else { continue };
            let members = entry
                .value
                .members
                .iter()
                .filter_map(|m| {
                    let exported = m.value.path.last()?.value.clone();
                    let bound = m.value.alias.as_ref().map_or_else(|| exported.clone(), |a| a.value.clone());
                    Some((exported, bound))
                })
                .collect();
            out.push(Imported { module: path.iter().map(|n| n.value.clone()).collect(), members });
        }
    }
    out
}

#[allow(deprecated)]
fn symbol(name: String, detail: Option<String>, kind: SymbolKind, range: lsp_types::Range, selection: lsp_types::Range, children: Vec<DocumentSymbol>) -> DocumentSymbol {
    DocumentSymbol { name, detail, kind, tags: None, deprecated: None, range, selection_range: selection, children: (!children.is_empty()).then_some(children) }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_owned()
}

/// The outline of `file` as hierarchical document symbols: functions, constants and types, the
/// members of a `has` body below their type, and imports.
pub fn document_symbols(file: &SourceFile, text: &str, index: &LineIndex) -> Vec<DocumentSymbol> {
    let range = |s: usize, e: usize| index.range(text, s, e);
    let mut out = Vec::new();
    for item in &file.items {
        let span = file.span_table.get(item.span_id);
        let whole = range(span.start(), span.end());
        match &item.value {
            Item::Binding(b) => {
                let name = file.span_table.get(b.name.span_id);
                let header = first_line(&text[span.start()..span.end()]);
                out.push(symbol(b.name.value.clone(), Some(header), binding_kind(b, false), whole, range(name.start(), name.end()), Vec::new()));
            }
            Item::Type(t) => {
                let name = file.span_table.get(t.name.span_id);
                let mut children = Vec::new();
                if let TypeBody::Has(members) = &t.body.value {
                    for member in members {
                        let member_span = file.span_table.get(member.span_id);
                        if let TypeMember::Binding(b) = &member.value {
                            let n = file.span_table.get(b.name.span_id);
                            let header = first_line(&text[member_span.start()..member_span.end()]);
                            children.push(symbol(b.name.value.clone(), Some(header), binding_kind(b, true), range(member_span.start(), member_span.end()), range(n.start(), n.end()), Vec::new()));
                        }
                    }
                }
                let header = first_line(&text[span.start()..span.end()]);
                out.push(symbol(t.name.value.clone(), Some(header), type_kind(&t.body.value), whole, range(name.start(), name.end()), children));
            }
            Item::Import(_) => {
                let line = first_line(&text[span.start()..span.end()]);
                out.push(symbol(line, None, SymbolKind::MODULE, whole, whole, Vec::new()));
            }
            _ => {}
        }
    }
    out
}
