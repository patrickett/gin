//! Shared analysis functions — the core logic that powers both ginlsp (LSP)
//! and ginmcp (MCP). These are stateless helpers that take parsed ASTs and
//! source text and return typed results.

use ast::FileAst;
use parser::ParseOutput;
use typeck::Ty;

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

/// Goto‑definition result.
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn byte_pos(source: &str, line: u32, character: u32) -> usize {
    typeck::position_to_byte_offset(source, line, character).unwrap_or(0)
}

fn byte_offset_to_position(offset: usize, source: &str) -> (u32, u32) {
    typeck::byte_offset_to_position(offset, source)
}

/// Parse optional scratchpad source and build a combined type environment.
/// Returns (scratchpad_ast, combined_ty_env) if scratchpad was provided.
fn build_scratchpad_env(
    main_ast: &FileAst,
    scratchpad_source: Option<&str>,
) -> (Option<FileAst>, typeck::TyEnv) {
    match scratchpad_source.filter(|s| !s.is_empty()) {
        Some(sp_source) => {
            let sp_po = parser::parse_source_full(sp_source);
            let env =
                typeck::TyEnv::from_multiple_file_asts(&[sp_po.ast.clone(), main_ast.clone()]);
            (Some(sp_po.ast), env)
        }
        None => (None, typeck::TyEnv::from_file_ast(main_ast)),
    }
}

/// Search for a symbol definition across multiple ASTs. Returns the span in the
/// AST where the definition was found, or None.
fn find_definition_multi(
    word: &str,
    main_ast: &FileAst,
    scratchpad_ast: Option<&FileAst>,
) -> Option<Span> {
    typeck::find_definition_span(main_ast, word)
        .or_else(|| scratchpad_ast.and_then(|sp| typeck::find_definition_span(sp, word)))
        .map(|r| Span {
            start: r.start,
            end: r.end,
        })
}

/// Compute hover text, searching the scratchpad AST if not found in the main AST.
fn hover_multi(
    source: &str,
    main_ast: &FileAst,
    scratchpad_ast: Option<&FileAst>,
    scratchpad_source: Option<&str>,
    byte_pos: usize,
) -> Option<String> {
    // Try the main AST first.
    if let Some(result) = typeck::hover_at(source, main_ast, byte_pos) {
        return Some(result);
    }
    // Not found — check if the symbol is defined in the scratchpad.
    let word = typeck::word_at_byte_offset(source, byte_pos)?;
    if let (Some(sp_ast), Some(sp_source)) = (scratchpad_ast, scratchpad_source) {
        if let Some(def_span) = typeck::find_definition_span(sp_ast, &word) {
            return typeck::hover_at(sp_source, sp_ast, def_span.start);
        }
    }
    None
}

/// Check whether a resolved `Ty` is a built-in structural primitive (Int, Float, Bool, Unit).
fn is_builtin_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Int { .. } | Ty::Float { .. } | Ty::Bool | Ty::Unit)
}

// ---------------------------------------------------------------------------
// Public query functions
// ---------------------------------------------------------------------------

/// Compute hover information at the given positions.
pub fn hover(
    po: &ParseOutput,
    source: &str,
    positions: &[Pos],
    path: Option<&str>,
    scratchpad: Option<&str>,
) -> Vec<HoverResult> {
    positions
        .iter()
        .map(|pos| {
            let bp = byte_pos(source, pos.line, pos.character);
            match path {
                Some(p) => {
                    let text = hover_at_with_imports(source, p, bp);
                    HoverResult { text, error: None }
                }
                None => {
                    let (sp_ast, _) = build_scratchpad_env(&po.ast, scratchpad);
                    let text = hover_multi(source, &po.ast, sp_ast.as_ref(), scratchpad, bp);
                    HoverResult { text, error: None }
                }
            }
        })
        .collect()
}

/// Compute goto-definition (single‑file, no import resolution).
pub fn goto_definition(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> GotoDefResult {
    let bp = byte_pos(source, line, character);
    let word = po
        .ast
        .word_at_byte(bp, source)
        .or_else(|| typeck::word_at_byte_offset(source, bp))
        .unwrap_or_default();
    let (sp_ast, _) = build_scratchpad_env(&po.ast, scratchpad);
    let rng = find_definition_multi(&word, &po.ast, sp_ast.as_ref());
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

/// Compute goto-definition with import resolution (filesystem‑aware).
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
    use ast::HasSpanId;
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
        }
    } else {
        let word = po
            .ast
            .word_at_byte(byte_pos, &source)
            .or_else(|| typeck::word_at_byte_offset(&source, byte_pos))?;
        typeck::find_definition_span(&po.ast, &word).map(|r| Span {
            start: r.start,
            end: r.end,
        })
    }
}

/// Compute hover text with import resolution (filesystem‑aware).
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
        }
    }

    typeck::hover_at(source, &po.ast, byte_pos)
}

/// Find all references to the symbol at the given position.
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
        .or_else(|| typeck::word_at_byte_offset(source, bp))
        .unwrap_or_default();
    let mut refs: Vec<Range> = typeck::find_references(&po.ast, &word)
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
        let sp_refs: Vec<Range> = typeck::find_references(&sp_po.ast, &word)
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

/// Get auto-completion suggestions for Gin source code.
pub fn completions(po: &ParseOutput, scratchpad: Option<&str>) -> Vec<CompletionItem> {
    let mut completions: Vec<CompletionItem> = typeck::completions_for_ast(&po.ast)
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
        let sp_items: Vec<CompletionItem> = typeck::completions_for_ast(&sp_po.ast)
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

/// Collect parse‑ and type‑check diagnostics.
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
    // Build type env from main + scratchpad so cross-file references resolve.
    let (_sp_ast, te) = build_scratchpad_env(&po.ast, scratchpad);
    for d in &typeck::analyze_file_with_ty_env(&po.ast, &te) {
        let sp = st.get(d.span_id);
        let (l, c) = byte_offset_to_position(sp.start, source);
        diags.push(DiagItem {
            line: l,
            character: c,
            message: d.message.clone(),
            category: format!("{:?}", d.category),
            code: format!("{:?}", d.code),
            source: "typeck",
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

/// List all top-level symbols (defs, functions, and tags) sorted alphabetically.
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
            .map(|p| format!("{}{}", n.as_str(), typeck::format_params(p)));
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

/// Format Gin source code.
pub fn format(source: &str) -> String {
    ginfmt::format(source)
}

/// Get signature help for a function call at the given position.
pub fn signature_help(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> Option<SignatureInfo> {
    let bp = byte_pos(source, line, character);
    let name = typeck::fn_call_at(&po.ast, bp).or_else(|| {
        scratchpad.and_then(|sp_source| {
            let sp_po = parser::parse_source_full(sp_source);
            typeck::fn_call_at(&sp_po.ast, bp)
        })
    });
    name.and_then(|n| {
        typeck::signature_for_fn(&po.ast, &n)
            .or_else(|| {
                scratchpad.and_then(|sp_source| {
                    let sp_po = parser::parse_source_full(sp_source);
                    typeck::signature_for_fn(&sp_po.ast, &n)
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
pub fn dot_type(
    po: &ParseOutput,
    source: &str,
    line: u32,
    character: u32,
    scratchpad: Option<&str>,
) -> Option<String> {
    let bp = byte_pos(source, line, character);
    let (_sp_ast, ty_env) = build_scratchpad_env(&po.ast, scratchpad);
    typeck::dot_type_at(source, &po.ast, &ty_env, bp).map(|ty| format!("{:?}", ty))
}

/// Get structured type information at the given positions.
pub fn type_at(
    po: &ParseOutput,
    source: &str,
    positions: &[Pos],
    scratchpad: Option<&str>,
) -> Vec<TypeAtResult> {
    positions
        .iter()
        .map(|pos| {
            let bp = byte_pos(source, pos.line, pos.character);
            let (sp_ast, ty_env) = build_scratchpad_env(&po.ast, scratchpad);
            let word = typeck::word_at_byte_offset(source, bp);
            let ty = word.as_ref().and_then(|w| {
                let interned = internment::Intern::from_ref(w);
                ty_env.lookup_tag(interned).cloned()
            });
            let origin = word.as_ref().map(|w| {
                let interned = internment::Intern::from_ref(w.as_str());
                if po.ast.tags().contains_key(&interned) || po.ast.defs().contains_key(&interned) {
                    "local"
                } else if sp_ast.as_ref().is_some_and(|sp| {
                    sp.tags()
                        .contains_key(&internment::Intern::from_ref(w.as_str()))
                }) {
                    "scratchpad"
                } else if let Some(resolved_ty) = ty_env.lookup_tag(interned) {
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
