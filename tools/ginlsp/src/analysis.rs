//! Shared analysis functions that power ginlsp (LSP).
//! These are stateless helpers that take parsed ASTs and
//! source text and return typed results.

use std::collections::HashMap;

use ast::ty::Ty;
use ast::typed::{transform_file, FileId};
use ast::{FileAst, TypedFileAst};
use internment::Intern;
use parser::ParseOutput;

/// A 0‑based position in source text.
#[derive(Debug, Clone, Copy)]
pub struct Pos {
    pub line: u32,
    pub character: u32,
}

/// A byte‑range span.
#[derive(Debug, Clone)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Result for a single position in a hover query.
#[derive(Debug)]
pub struct HoverResult {
    pub text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
pub struct GotoDefResult {
    pub start_line: Option<u32>,
    pub start_character: Option<u32>,
    pub end_line: Option<u32>,
    pub end_character: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct Range {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone)]
pub struct CompletionItem {
    pub label: String,
    pub kind: String,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiagItem {
    pub line: u32,
    pub character: u32,
    pub message: String,
    pub category: String,
    pub code: String,
    pub source: &'static str,
}

#[derive(Debug, Clone)]
pub struct SymbolItem {
    pub name: String,
    pub kind: String,
    pub detail: Option<String>,
    pub signature: Option<String>,
    pub private: bool,
}

#[derive(Debug, Clone)]
pub struct SignatureInfo {
    pub function: String,
    pub label: String,
    pub params: Vec<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TypeAtResult {
    pub origin: String,
    pub ty: Option<serde_json::Value>,
}

fn byte_pos(source: &str, line: u32, character: u32) -> usize {
    ast::position_to_byte_offset(source, line, character).unwrap_or(0)
}

fn byte_offset_to_position(offset: usize, source: &str) -> (u32, u32) {
    ast::byte_offset_to_position(offset, source)
}

fn to_typed_ast(ast: &FileAst) -> TypedFileAst {
    transform_file(ast.clone(), FileId(0))
}

pub fn build_tag_types_map(typed: &TypedFileAst) -> HashMap<Intern<String>, Ty> {
    typed
        .tag_types
        .iter()
        .map(|(id, ty)| (id.0, ty.clone()))
        .collect()
}

/// Check whether a resolved `Ty` is a built-in structural primitive (Int, Float, Bool, Unit).
fn is_builtin_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Int { .. } | Ty::Float { .. } | Ty::Bool | Ty::Unit)
}

pub fn hover(
    po: &ParseOutput,
    source: &str,
    positions: &[Pos],
    path: Option<&str>,
    scratchpad: Option<&str>,
) -> Vec<HoverResult> {
    let typed = to_typed_ast(&po.ast);
    positions
        .iter()
        .map(|pos| {
            let text = match path {
                Some(p) => {
                    hover_at_with_imports(source, p, byte_pos(source, pos.line, pos.character))
                }
                None => {
                    let result = typed.hover_at(source, pos.line, pos.character);
                    if result.is_some() {
                        result
                    } else {
                        scratchpad.filter(|s| !s.is_empty()).and_then(|sp_src| {
                            let sp_po = parser::parse_source_full(sp_src);
                            let sp_typed = to_typed_ast(&sp_po.ast);
                            let def_span =
                                sp_typed.definition_span(source, pos.line, pos.character);
                            def_span.and_then(|(start_byte, _end_byte)| {
                                let (l, c) = ast::byte_offset_to_position(start_byte, source);
                                typed.hover_at(source, l, c)
                            })
                        })
                    }
                }
            };
            HoverResult { text, error: None }
        })
        .collect()
}

/// Single‑file, no import resolution.
pub fn goto_definition(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> GotoDefResult {
    let typed = to_typed_ast(&po.ast);
    let rng = typed
        .definition_span(source, line, character)
        .or_else(|| {
            scratchpad.filter(|s| !s.is_empty()).and_then(|sp_src| {
                let sp_po = parser::parse_source_full(sp_src);
                let sp_typed = to_typed_ast(&sp_po.ast);
                sp_typed.definition_span(source, line, character)
            })
        })
        .map(|(start, end)| Span { start, end });
    match rng {
        Some(r) => {
            let (a, b) = byte_offset_to_position(r.start, source);
            let (c, d) = byte_offset_to_position(r.end, source);
            GotoDefResult {
                start_line: Some(a),
                start_character: Some(b),
                end_line: Some(c),
                end_character: Some(d),
            }
        }
        None => GotoDefResult::default(),
    }
}

/// Filesystem‑aware import resolution.
pub fn goto_definition_with_imports(
    source: &str,
    path: &str,
    line: u32,
    character: u32,
) -> GotoDefResult {
    let bp = byte_pos(source, line, character);
    let result = resolve_symbol_def_span(path, bp);
    match result {
        Some(r) => {
            let (a, b) = byte_offset_to_position(r.start, source);
            let (c, d) = byte_offset_to_position(r.end, source);
            GotoDefResult {
                start_line: Some(a),
                start_character: Some(b),
                end_line: Some(c),
                end_character: Some(d),
            }
        }
        None => GotoDefResult::default(),
    }
}

/// Resolve the import at `byte_pos` in `source` using filesystem
/// import resolution and return the definition span.
pub fn resolve_symbol_def_span(file_path: &str, byte_pos: usize) -> Option<Span> {
    use ast::ImportSource;

    let source = std::fs::read_to_string(file_path).ok()?;
    let po = parser::parse_source_full(&source);

    if let Some(target) = resolve::resolve_import_at(&po.ast, &source, byte_pos) {
        match target {
            resolve::ImportTarget::DepRoot { dep_name } => {
                resolve::resolve_dep_dir(std::path::Path::new(file_path), &dep_name)?;
                for import in po.ast.uses() {
                    for mi in &import.0 {
                        if let ImportSource::Package(mp) = &mi.source {
                            let span_table = po.ast.span_table();
                            let span = span_table.get(mp.span_id());
                            if byte_pos >= span.start && byte_pos <= span.end {
                                return Some(Span {
                                    start: span.start,
                                    end: span.start,
                                });
                            }
                        }
                    }
                }
                None
            }
            resolve::ImportTarget::DepSymbol { dep_name, symbol }
            | resolve::ImportTarget::BodySymbol { dep_name, symbol } => {
                let r = resolve::resolve_symbol_def_span(
                    std::path::Path::new(file_path),
                    &dep_name,
                    &symbol,
                    &resolve::default_file_reader,
                )?;
                Some(Span {
                    start: r.start,
                    end: r.end,
                })
            }
            resolve::ImportTarget::LocalBundleSymbol { local_path, symbol } => {
                let r = resolve::resolve_local_symbol_def_span(
                    std::path::Path::new(file_path),
                    &local_path,
                    &symbol,
                    &resolve::default_file_reader,
                )?;
                Some(Span {
                    start: r.start,
                    end: r.end,
                })
            }
            resolve::ImportTarget::CurrentModuleSymbol { symbol } => {
                let r = resolve::resolve_current_module_def_span(
                    std::path::Path::new(file_path),
                    &symbol,
                    &resolve::default_file_reader,
                )?;
                Some(Span {
                    start: r.start,
                    end: r.end,
                })
            }
        }
    } else {
        let word = po
            .ast
            .word_at_byte(byte_pos, &source)
            .or_else(|| ast::word_at_byte_offset(&source, byte_pos))?;
        ast::hover::definition_span(&po.ast, &word).map(|r| Span {
            start: r.start,
            end: r.end,
        })
    }
}

/// Filesystem‑aware import resolution.
pub fn hover_at_with_imports(source: &str, file_path: &str, byte_pos: usize) -> Option<String> {
    let po = parser::parse_source_full(source);

    if let Some(target) = resolve::resolve_import_at(&po.ast, source, byte_pos) {
        match target {
            resolve::ImportTarget::DepRoot { dep_name } => {
                return resolve::resolve_dep_hover(std::path::Path::new(file_path), &dep_name);
            }
            resolve::ImportTarget::DepSymbol { dep_name, symbol }
            | resolve::ImportTarget::BodySymbol { dep_name, symbol } => {
                return resolve::resolve_symbol_hover(
                    std::path::Path::new(file_path),
                    &dep_name,
                    &symbol,
                    &resolve::default_file_reader,
                );
            }
            resolve::ImportTarget::LocalBundleSymbol { local_path, symbol } => {
                return resolve::resolve_local_symbol_hover(
                    std::path::Path::new(file_path),
                    &local_path,
                    &symbol,
                    &resolve::default_file_reader,
                );
            }
            resolve::ImportTarget::CurrentModuleSymbol { symbol } => {
                return resolve::resolve_current_module_hover(
                    std::path::Path::new(file_path),
                    &symbol,
                    &resolve::default_file_reader,
                );
            }
        }
    }

    let typed = to_typed_ast(&po.ast);
    let (line, character) = ast::byte_offset_to_position(byte_pos, source);
    typed.hover_at(source, line, character)
}

pub fn references(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> Vec<Range> {
    let bp = byte_pos(source, line, character);
    let word = po
        .ast
        .word_at_byte(bp, source)
        .or_else(|| ast::word_at_byte_offset(source, bp))
        .unwrap_or_default();
    let mut refs: Vec<Range> = ast::hover::find_references(&po.ast, &word)
        .into_iter()
        .map(|r| {
            let (sl, sc) = byte_offset_to_position(r.start, source);
            let (el, ec) = byte_offset_to_position(r.end, source);
            Range {
                start: Pos {
                    line: sl,
                    character: sc,
                },
                end: Pos {
                    line: el,
                    character: ec,
                },
            }
        })
        .collect();
    // Also search the scratchpad if provided.
    if let Some(sp_source) = scratchpad.filter(|s| !s.is_empty()) {
        let sp_po = parser::parse_source_full(sp_source);
        let sp_refs: Vec<Range> = ast::hover::find_references(&sp_po.ast, &word)
            .into_iter()
            .map(|r| {
                let (sl, sc) = byte_offset_to_position(r.start, sp_source);
                let (el, ec) = byte_offset_to_position(r.end, sp_source);
                Range {
                    start: Pos {
                        line: sl,
                        character: sc,
                    },
                    end: Pos {
                        line: el,
                        character: ec,
                    },
                }
            })
            .collect();
        refs.extend(sp_refs);
    }
    refs
}

pub fn completions(po: &ParseOutput, scratchpad: Option<&str>) -> Vec<CompletionItem> {
    let mut completions: Vec<CompletionItem> = ast::completions::completions_for_ast(&po.ast)
        .into_iter()
        .map(|c| CompletionItem {
            label: c.label,
            kind: format!("{:?}", c.kind),
            detail: c.detail,
            documentation: c.documentation,
        })
        .collect();
    // Merge scratchpad completions.
    if let Some(sp_source) = scratchpad.filter(|s| !s.is_empty()) {
        let sp_po = parser::parse_source_full(sp_source);
        let sp_items: Vec<CompletionItem> = ast::completions::completions_for_ast(&sp_po.ast)
            .into_iter()
            .map(|c| CompletionItem {
                label: c.label,
                kind: format!("{:?}", c.kind),
                detail: c.detail,
                documentation: c.documentation,
            })
            .collect();
        completions.extend(sp_items);
    }
    completions
}

pub fn diagnostics(po: &ParseOutput, source: &str, scratchpad: Option<&str>) -> Vec<DiagItem> {
    let st = po.ast.span_table();
    let mut diags: Vec<DiagItem> = Vec::new();
    for d in &po.symptoms {
        let sp = st.get(d.span_id);
        let (l, c) = byte_offset_to_position(sp.start, source);
        diags.push(DiagItem {
            line: l,
            character: c,
            message: d.message.clone(),
            category: format!("{:?}", d.category),
            code: format!("{:?}", d.code),
            source: "parse",
        });
    }

    // Also include scratchpad parse symptoms.
    if let Some(sp_source) = scratchpad.filter(|s| !s.is_empty()) {
        let sp_po = parser::parse_source_full(sp_source);
        let sp_st = sp_po.ast.span_table();
        for d in &sp_po.symptoms {
            let sp = sp_st.get(d.span_id);
            let (l, c) = byte_offset_to_position(sp.start, sp_source);
            diags.push(DiagItem {
                line: l,
                character: c,
                message: d.message.clone(),
                category: format!("{:?}", d.category),
                code: format!("{:?}", d.code),
                source: "parse.scratchpad",
            });
        }
    }
    diags
}

/// This is the typed-AST equivalent of `diagnostics()`, producing `DiagItem`s
/// from `TypeSymptom`s on each expression node in the typed expression arena.
pub fn typed_diagnostics(po: &ParseOutput, source: &str) -> Vec<DiagItem> {
    let st = po.ast.span_table();
    let mut diags: Vec<DiagItem> = Vec::new();

    // Parse diagnostics (same as regular path).
    for d in &po.symptoms {
        let sp = st.get(d.span_id);
        let (l, c) = byte_offset_to_position(sp.start, source);
        diags.push(DiagItem {
            line: l,
            character: c,
            message: d.message.clone(),
            category: format!("{:?}", d.category),
            code: format!("{:?}", d.code),
            source: "parse",
        });
    }

    use diagnostic::DiagnosticLike;

    // Transform to TypedFileAst and collect type flaws via all_flaws().
    let typed = ast::typed::transform_file(po.ast.clone(), ast::typed::FileId(0));
    for (expr_id, flaw) in typed.all_flaws() {
        let idx = expr_id.as_usize();
        let span_id = typed.exprs.span[idx];
        let sp = typed.span_table.get(span_id);
        let (l, c) = byte_offset_to_position(sp.start, source);
        let d = flaw.clone().into_diagnostic(span_id);
        let category = d.category.as_str().to_string();
        let code = d.error_code().to_string();
        diags.push(DiagItem {
            line: l,
            character: c,
            message: d.message,
            category,
            code,
            source: "typeck.typed",
        });
    }

    diags
}

pub fn typed_symbols(po: &ParseOutput, scratchpad: Option<&str>) -> Vec<SymbolItem> {
    let typed = ast::typed::transform_file(po.ast.clone(), ast::typed::FileId(0));
    let mut syms: Vec<SymbolItem> = Vec::new();

    // Tag symbols.
    for (tag_id, tag) in &typed.tags {
        syms.push(SymbolItem {
            name: tag_id.0.as_str().to_string(),
            kind: "tag".to_string(),
            detail: Some(format!("{:?}", tag.resolved_ty)),
            signature: None,
            private: typed.private_tags.contains(tag_id),
        });
    }

    // Def symbols.
    for (def_id, bind) in &typed.defs {
        let sig = if !bind.params.is_empty() {
            let params: Vec<String> = bind
                .params
                .iter()
                .map(|(n, t)| format!("{}: {:?}", n.as_str(), t))
                .collect();
            Some(format!("({})", params.join(", ")))
        } else {
            None
        };
        syms.push(SymbolItem {
            name: def_id.0.as_str().to_string(),
            kind: if !bind.params.is_empty() {
                "function"
            } else {
                "bind"
            }
            .to_string(),
            detail: Some(format!("{:?}", bind.return_type)),
            signature: sig,
            private: typed.private_defs.contains(def_id),
        });
    }

    if let Some(sp_source) = scratchpad.filter(|s| !s.is_empty()) {
        let sp_po = parser::parse_source_full(sp_source);
        let sp_syms = collect_symbols(&sp_po.ast);
        syms.extend(sp_syms);
    }

    syms.sort_by(|a, b| a.name.cmp(&b.name));
    syms
}

pub fn typed_references(po: &ParseOutput, source: &str, line: u32, character: u32) -> Vec<Range> {
    let typed = ast::typed::transform_file(po.ast.clone(), ast::typed::FileId(0));
    let mut refs: Vec<Range> = Vec::new();

    // Find the target symbol at the given position.
    let Some(expr_id) = typed.expr_at_source_pos(source, line, character) else {
        return refs;
    };
    let Some(expr_ref) = typed.expr(expr_id) else {
        return refs;
    };

    // Determine the target name.
    let target_name: Option<String> = match expr_ref.kind {
        ast::typed::TypedExprKind::Bind { name, .. } => Some(name.as_str().to_string()),
        ast::typed::TypedExprKind::FnCall { target, .. } => Some(target.0.as_str().to_string()),
        _ => None,
    };
    let Some(target_name) = target_name else {
        return refs;
    };

    // Walk the expression arena and find all references.
    for i in 0..typed.exprs.kind.len() {
        let kind = &typed.exprs.kind[i];
        let matches = match kind {
            ast::typed::TypedExprKind::Bind { name, .. } => name.as_str() == target_name,
            ast::typed::TypedExprKind::FnCall { target, .. } => target.0.as_str() == target_name,
            _ => false,
        };
        if matches {
            let span_id = typed.exprs.span[i];
            let sp = typed.span_table.get(span_id);
            let start = byte_offset_to_position(sp.start, source);
            let end = byte_offset_to_position(sp.end, source);
            refs.push(Range {
                start: Pos {
                    line: start.0,
                    character: start.1,
                },
                end: Pos {
                    line: end.0,
                    character: end.1,
                },
            });
        }
    }

    refs
}

pub fn symbols(po: &ParseOutput, scratchpad: Option<&str>) -> Vec<SymbolItem> {
    let mut syms = collect_symbols(&po.ast);
    if let Some(sp_source) = scratchpad.filter(|s| !s.is_empty()) {
        let sp_po = parser::parse_source_full(sp_source);
        let sp_syms = collect_symbols(&sp_po.ast);
        syms.extend(sp_syms);
    }
    syms.sort_by(|a, b| a.name.cmp(&b.name));
    syms
}

fn collect_symbols(a: &FileAst) -> Vec<SymbolItem> {
    let mut syms: Vec<SymbolItem> = Vec::new();
    for (n, d) in a.tags() {
        syms.push(SymbolItem {
            name: n.as_str().into(),
            kind: "tag".into(),
            detail: Some(d.to_string()),
            signature: None,
            private: a.private_tags().contains(n),
        });
    }
    for (n, b) in a.defs() {
        let sig = b
            .params()
            .as_ref()
            .map(|p| format!("{}{}", n.as_str(), ast::completions::format_params(p)));
        syms.push(SymbolItem {
            name: n.as_str().into(),
            kind: if b.params().is_some() {
                "function".into()
            } else {
                "bind".into()
            },
            detail: None,
            signature: sig,
            private: a.private_defs().contains(n),
        });
    }
    syms
}

pub fn typed_completions(po: &ParseOutput, _line: u32, _character: u32) -> Vec<CompletionItem> {
    let typed = ast::typed::transform_file(po.ast.clone(), ast::typed::FileId(0));
    let mut items: Vec<CompletionItem> = Vec::new();

    // Add all tags.
    for tag_id in typed.tags.keys() {
        items.push(CompletionItem {
            label: tag_id.0.as_str().to_string(),
            kind: "tag".to_string(),
            detail: Some("type".to_string()),
            documentation: None,
        });
    }

    // Add all defs.
    for (def_id, bind) in &typed.defs {
        let kind = if !bind.params.is_empty() {
            "function"
        } else {
            "bind"
        };
        items.push(CompletionItem {
            label: def_id.0.as_str().to_string(),
            kind: kind.to_string(),
            detail: Some(format!("{:?}", bind.return_type)),
            documentation: None,
        });
    }

    items
}

pub fn typed_signature_help(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
) -> Option<SignatureInfo> {
    let typed = ast::typed::transform_file(po.ast.clone(), ast::typed::FileId(0));
    let bp = byte_pos(source, line, character);
    let word = ast::word_at_byte_offset(source, bp)?;
    let word_str = word.clone();
    let word_interned = internment::Intern::new(word);

    // Look up the word in typed defs.
    let def_id = ast::typed::DefId(word_interned);
    let bind = typed.defs.get(&def_id)?;

    let params: Vec<String> = bind
        .params
        .iter()
        .map(|(n, t)| format!("{}: {:?}", n.as_str(), t))
        .collect();
    let label = format!("{}({})", word_str, params.join(", "));

    Some(SignatureInfo {
        function: word_str,
        label,
        params: params.clone(),
        documentation: bind.doc_comment.as_ref().map(|d| d.value.clone()),
    })
}

/// Format Gin source code.
pub fn format(source: &str) -> String {
    ginfmt::format(source)
}

pub fn signature_help(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> Option<SignatureInfo> {
    let bp = byte_pos(source, line, character);
    let name = ast::completions::fn_call_at(&po.ast, bp).or_else(|| {
        scratchpad.and_then(|sp_source| {
            let sp_po = parser::parse_source_full(sp_source);
            ast::completions::fn_call_at(&sp_po.ast, bp)
        })
    });
    name.and_then(|n| {
        ast::completions::signature_for_fn(&po.ast, &n)
            .or_else(|| {
                scratchpad.and_then(|sp_source| {
                    let sp_po = parser::parse_source_full(sp_source);
                    ast::completions::signature_for_fn(&sp_po.ast, &n)
                })
            })
            .map(|info| SignatureInfo {
                function: n,
                label: info.label,
                params: info.params,
                documentation: info.documentation,
            })
    })
}

/// Resolve the type of an expression after a dot (`.`).
pub fn dot_type(po: &ParseOutput, source: &str, line: u32, character: u32) -> Option<String> {
    let bp = byte_pos(source, line, character);
    ast::hover::dot_type_at(source, &po.ast, bp).map(|ty| format!("{:?}", ty))
}

pub fn type_at(
    po: &ParseOutput,
    source: &str,
    positions: &[Pos],
    scratchpad: Option<&str>,
) -> Vec<TypeAtResult> {
    // Transform main AST via typed pipeline once for all positions.
    let typed = to_typed_ast(&po.ast);
    let tag_types = build_tag_types_map(&typed);
    // Build scratchpad-resolved tag_types for origin detection.
    let sp_tag_types: Option<HashMap<Intern<String>, Ty>> =
        scratchpad.filter(|s| !s.is_empty()).map(|sp_src| {
            let sp_po = parser::parse_source_full(sp_src);
            let sp_typed = to_typed_ast(&sp_po.ast);
            build_tag_types_map(&sp_typed)
        });
    positions
        .iter()
        .map(|pos| {
            let bp = byte_pos(source, pos.line, pos.character);
            let word = ast::word_at_byte_offset(source, bp);
            let ty = word.as_ref().and_then(|w| {
                let interned = Intern::from_ref(w);
                tag_types.get(&interned).cloned()
            });
            let origin = word.as_ref().map(|w| {
                let interned = Intern::from_ref(w.as_str());
                if po.ast.tags().contains_key(&interned) || po.ast.defs().contains_key(&interned) {
                    "local"
                } else if sp_tag_types
                    .as_ref()
                    .is_some_and(|sp| sp.contains_key(&Intern::from_ref(w.as_str())))
                {
                    "scratchpad"
                } else if let Some(resolved_ty) = tag_types.get(&interned) {
                    if is_builtin_ty(resolved_ty) {
                        "builtin"
                    } else {
                        "imported"
                    }
                } else {
                    "unknown"
                }
                .to_string()
            });
            TypeAtResult {
                origin: origin.unwrap_or_default(),
                ty: ty.as_ref().map(crate::json::ty_to_json),
            }
        })
        .collect()
}
