//! Completions and signature-help (semantic layer, no LSP types).

use crate::ty::Ty;
use ast::{
    Bind, BindValue, Expr, FileAst, LoopEnum, ModPath, ParameterKind, Parameters, SpanId,
    SpanTable, WhenArm,
};
use ast::{ConstValue, HashFloat};
use ast_format::type_expr::ExprFormatExt;
use internment::Intern;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum CompletionKind {
    Function,
    Variable,
    Tag,
    Keyword,
}

#[derive(Debug, Clone)]
pub struct CompletionCandidate {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    /// Custom text to insert (None = use label).
    pub insert_text: Option<String>,
    /// Whether insert_text uses LSP snippet syntax (tabstops like `${1:}`).
    pub is_snippet: bool,
}

impl CompletionCandidate {
    pub fn new(label: &str, kind: CompletionKind) -> Self {
        Self {
            label: label.to_string(),
            kind,
            detail: None,
            documentation: None,
            insert_text: None,
            is_snippet: false,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn snippet(label: &str, snippet: &str, kind: CompletionKind) -> Self {
        Self {
            label: label.to_string(),
            kind,
            detail: None,
            documentation: None,
            insert_text: Some(snippet.to_string()),
            is_snippet: true,
        }
    }
}

/// Describes what kind of code region the cursor is in.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorContext {
    /// Top level — no enclosing bind body.
    Root,
    /// Inside a `use ...` statement.
    Use,
    /// Inside a bind body (general expression position).
    BindBody(ScopeInfo),
    /// Inside an `if … is …` expression.
    InsideIf(ScopeInfo),
    /// Inside a `when …` expression.
    InsideWhen(ScopeInfo),
    /// Inside a `while` or `for … in …` loop body.
    InsideLoop(ScopeInfo),
}

/// Variables visible from the cursor position inward.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScopeInfo {
    /// Parameter names of the enclosing function / bind.
    pub param_names: Vec<String>,
    /// Local variable names (introduced via `name := …` or `name: …`) visible
    /// at the cursor position.
    pub local_var_names: Vec<String>,
}

/// Detect the cursor context from the AST and source text.
pub fn detect_cursor_context(ast: &FileAst, source: &str, byte_pos: usize) -> CursorContext {
    // 1. On a `use` line?
    if is_on_use_line(source, byte_pos) {
        return CursorContext::Use;
    }

    // 2. Inside a bind body? (via expr_at_byte)
    if let Some((expr, _)) = ast.expr_at_byte(byte_pos) {
        let scope = collect_scope_info(ast, byte_pos);
        return match expr {
            Expr::If(_) => CursorContext::InsideIf(scope),
            Expr::When(_) => CursorContext::InsideWhen(scope),
            Expr::Loop(_) => CursorContext::InsideLoop(scope),
            _ => CursorContext::BindBody(scope),
        };
    }

    // 3. Cursor on whitespace — use indentation heuristic.
    if is_indented_line(source, byte_pos) {
        let scope = collect_scope_info(ast, byte_pos);
        // Try to detect whether we're inside an if/when/loop by looking at
        // the enclosing expression via a slightly broader span query.
        if let Some(sub_context) = detect_nested_block_context(ast, byte_pos) {
            return match sub_context {
                NestedBlock::If => CursorContext::InsideIf(scope),
                NestedBlock::When => CursorContext::InsideWhen(scope),
                NestedBlock::Loop => CursorContext::InsideLoop(scope),
                NestedBlock::Bind => CursorContext::BindBody(scope),
            };
        }
        return CursorContext::BindBody(scope);
    }

    // 4. Otherwise, root level.
    CursorContext::Root
}

fn is_on_use_line(source: &str, byte_pos: usize) -> bool {
    let line_start = source[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &source[line_start..];
    line.trim_start().starts_with("use ")
}

fn is_indented_line(source: &str, byte_pos: usize) -> bool {
    let line_start = source[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if line_start >= source.len() {
        return false;
    }
    let c = source.as_bytes()[line_start];
    c == b' ' || c == b'\t'
}

enum NestedBlock {
    If,
    When,
    Loop,
    Bind,
}

/// When the cursor is on whitespace between expressions, walk the AST to find
/// the nearest enclosing block expression that spans the cursor.
fn detect_nested_block_context(ast: &FileAst, byte_pos: usize) -> Option<NestedBlock> {
    for bind in ast.defs.values() {
        let result = detect_block_in_bind_value(&ast.span_table, bind, byte_pos);
        if result.is_some() {
            return result;
        }
    }
    None
}

fn detect_block_in_bind_value(st: &SpanTable, bind: &Bind, byte_pos: usize) -> Option<NestedBlock> {
    match &bind.value {
        BindValue::Body { exprs, ret } => {
            // Check top-level exprs and the return expr.
            for expr in exprs {
                if st.contains(expr.span_id, byte_pos) {
                    return detect_block_in_expr(st, &expr.value, byte_pos);
                }
            }
            if st.contains(ret.span_id, byte_pos) {
                return Some(NestedBlock::Bind);
            }
            // Also check spans between expressions: if the cursor falls
            // between two body expressions (within the body's overall range)
            // but not inside any single expression, we're still in the bind.
            if let Some(first) = exprs.first()
                && let Some(last) = exprs.last()
            {
                let first_span = st.get(first.span_id);
                let last_span = st.get(last.span_id);
                let body_start = first_span.start();
                let body_end = (last_span.end()).max(st.get(ret.span_id).end());
                if byte_pos >= body_start && byte_pos <= body_end {
                    return Some(NestedBlock::Bind);
                }
            }
            None
        }
        BindValue::Expr(expr) => {
            if st.contains(expr.span_id, byte_pos) {
                detect_block_in_expr(st, &expr.value, byte_pos)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn detect_block_in_expr(st: &SpanTable, expr: &Expr, byte_pos: usize) -> Option<NestedBlock> {
    match expr {
        Expr::Bind(b) => detect_block_in_bind_value(st, b, byte_pos),
        // Walk into compound expressions to find the innermost block.
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    if st.contains(arg.span_id, byte_pos) {
                        return detect_block_in_expr(st, &arg.value, byte_pos);
                    }
                }
            }
            None
        }
        Expr::Binary(bin) => {
            if st.contains(bin.lhs.span_id, byte_pos) {
                detect_block_in_expr(st, &bin.lhs.value, byte_pos)
            } else if st.contains(bin.rhs.span_id, byte_pos) {
                detect_block_in_expr(st, &bin.rhs.value, byte_pos)
            } else {
                None
            }
        }
        Expr::When(w) => {
            // Check arms for nested blocks first.
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        let mut nested = None;
                        let _ = ast::folder::walk_condition_typed_exprs(condition, &mut |child| {
                            if nested.is_none() && st.contains(child.span_id, byte_pos) {
                                nested = detect_block_in_expr(st, &child.value, byte_pos);
                            }
                            std::ops::ControlFlow::Continue(())
                        });
                        if nested.is_some() {
                            return nested;
                        }
                        if st.contains(body.span_id, byte_pos) {
                            return detect_block_in_expr(st, &body.value, byte_pos);
                        }
                    }
                    WhenArm::Is { body, .. } => {
                        if st.contains(body.span_id, byte_pos) {
                            return detect_block_in_expr(st, &body.value, byte_pos);
                        }
                    }
                    WhenArm::Else(body, _) => {
                        if st.contains(body.span_id, byte_pos) {
                            return detect_block_in_expr(st, &body.value, byte_pos);
                        }
                    }
                }
            }
            Some(NestedBlock::When)
        }
        Expr::If(ifx) => {
            for e in &ifx.body {
                if st.contains(e.span_id, byte_pos) {
                    return detect_block_in_expr(st, &e.value, byte_pos);
                }
            }
            if let Some(ret_expr) = &ifx.ret.value
                && st.contains(ret_expr.span_id, byte_pos)
            {
                return detect_block_in_expr(st, &ret_expr.value, byte_pos);
            }
            Some(NestedBlock::If)
        }
        Expr::Loop(loop_val) => match loop_val {
            LoopEnum::While(w) => {
                for e in &w.exprs {
                    if st.contains(e.span_id, byte_pos) {
                        return detect_block_in_expr(st, &e.value, byte_pos);
                    }
                }
                Some(NestedBlock::Loop)
            }
            LoopEnum::ForIn(f) => {
                for e in &f.exprs {
                    if st.contains(e.span_id, byte_pos) {
                        return detect_block_in_expr(st, &e.value, byte_pos);
                    }
                }
                Some(NestedBlock::Loop)
            }
        },
        _ => None,
    }
}

/// Collect parameter names and local variable names visible at `byte_pos`
/// by walking the enclosing bind body and gathering declarations before the cursor.
fn collect_scope_info(ast: &FileAst, byte_pos: usize) -> ScopeInfo {
    let mut info = ScopeInfo::default();

    // Find the enclosing top-level bind.
    let bind = match find_containing_bind(ast, byte_pos) {
        Some(b) => b,
        None => return info,
    };

    // Collect parameter names.
    if let Some(params) = &bind.params {
        for (name, _kind) in params {
            info.param_names.push(name.as_str().to_string());
        }
    }

    // Collect local variable declarations from the body before byte_pos.
    collect_locals_from_bind_value(&ast.span_table, &bind.value, byte_pos, &mut info);

    info
}

/// Find the top-level bind whose body contains `byte_pos`.
fn find_containing_bind(ast: &FileAst, byte_pos: usize) -> Option<&Bind> {
    let st = &ast.span_table;
    for bind in ast.defs.values() {
        match &bind.value {
            BindValue::Body { exprs, ret } => {
                if let Some(first) = exprs.first() {
                    let first_span = st.get(first.span_id);
                    let body_end = if let Some(last) = exprs.last() {
                        st.get(last.span_id).end()
                    } else {
                        first_span.end()
                    };
                    let body_end = body_end.max(st.get(ret.span_id).end());
                    if byte_pos >= first_span.start() && byte_pos <= body_end {
                        return Some(bind);
                    }
                } else if st.get(ret.span_id).contains(byte_pos) {
                    return Some(bind);
                }
            }
            BindValue::Expr(expr) if st.contains(expr.span_id, byte_pos) => {
                return Some(bind);
            }
            _ => {}
        }
    }
    None
}

/// Walk expression list in order, collecting local `Expr::Bind` names before `byte_pos`.
/// Recurses into nested blocks that contain `byte_pos`.
fn collect_locals_from_bind_value(
    st: &SpanTable,
    value: &BindValue,
    byte_pos: usize,
    info: &mut ScopeInfo,
) {
    match value {
        BindValue::Body { exprs, ret: _ } => {
            for expr in exprs {
                if (st.get(expr.span_id).start()) >= byte_pos {
                    break;
                }
                collect_locals_from_expr(st, &expr.value, byte_pos, info);
                // If the cursor is inside this expression, also walk into nested blocks.
                if st.contains(expr.span_id, byte_pos) {
                    collect_locals_nested(st, &expr.value, byte_pos, info);
                }
            }
        }
        BindValue::Expr(expr) if st.contains(expr.span_id, byte_pos) => {
            collect_locals_from_expr(st, &expr.value, byte_pos, info);
            collect_locals_nested(st, &expr.value, byte_pos, info);
        }
        _ => {}
    }
}

/// Collect local variable names from a single expression — handles `Expr::Bind` at the top level.
fn collect_locals_from_expr(_st: &SpanTable, expr: &Expr, _byte_pos: usize, info: &mut ScopeInfo) {
    if let Expr::Bind(b) = expr {
        info.local_var_names.push(b.name.as_str().to_string());
    }
}

/// Walk into nested blocks (if/for/while/when) that contain the cursor and collect their locals.
fn collect_locals_nested(st: &SpanTable, expr: &Expr, byte_pos: usize, info: &mut ScopeInfo) {
    match expr {
        Expr::Bind(b) => {
            collect_locals_from_bind_value(st, &b.value, byte_pos, info);
        }
        Expr::If(ifx) => {
            for e in &ifx.body {
                if (st.get(e.span_id).start()) >= byte_pos {
                    break;
                }
                collect_locals_from_expr(st, &e.value, byte_pos, info);
                if st.contains(e.span_id, byte_pos) {
                    collect_locals_nested(st, &e.value, byte_pos, info);
                }
            }
        }
        Expr::When(w) => {
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond { body, .. }
                    | WhenArm::Is { body, .. }
                    | WhenArm::Else(body, _) => {
                        if st.contains(body.span_id, byte_pos) {
                            collect_locals_from_expr(st, &body.value, byte_pos, info);
                            collect_locals_nested(st, &body.value, byte_pos, info);
                        }
                    }
                }
            }
        }
        Expr::Loop(loop_val) => match loop_val {
            LoopEnum::While(w) => {
                for e in &w.exprs {
                    if (st.get(e.span_id).start()) >= byte_pos {
                        break;
                    }
                    collect_locals_from_expr(st, &e.value, byte_pos, info);
                    if st.contains(e.span_id, byte_pos) {
                        collect_locals_nested(st, &e.value, byte_pos, info);
                    }
                }
            }
            LoopEnum::ForIn(f) => {
                for e in &f.exprs {
                    if (st.get(e.span_id).start()) >= byte_pos {
                        break;
                    }
                    collect_locals_from_expr(st, &e.value, byte_pos, info);
                    if st.contains(e.span_id, byte_pos) {
                        collect_locals_nested(st, &e.value, byte_pos, info);
                    }
                }
            }
        },
        _ => {}
    }
}

/// Extension trait providing completion methods on [`FileAst`].
pub trait FileAstCompletionExt {
    fn completions_for_context(
        &self,
        source: &str,
        byte_pos: usize,
        tag_types: Option<&HashMap<Intern<String>, Ty>>,
    ) -> Vec<CompletionCandidate>;
}

impl FileAstCompletionExt for FileAst {
    fn completions_for_context(
        &self,
        source: &str,
        byte_pos: usize,
        tag_types: Option<&HashMap<Intern<String>, Ty>>,
    ) -> Vec<CompletionCandidate> {
        // 1. Literal-union completions (cursor after `TypeName:` value area).
        if let Some(tag_types) = tag_types {
            let literal = literal_union_completions_at(source, byte_pos, tag_types);
            if !literal.is_empty() {
                return literal;
            }
        }

        // 2. Dot-after-type completions (cursor after `TypeName.`).
        if let Some(dot_items) = dot_completions_for_source(source, byte_pos, tag_types) {
            return dot_items;
        }

        // 3. Context-aware completions.
        let context = detect_cursor_context(self, source, byte_pos);
        completions_for_context_inner(self, context)
    }
}

fn completions_for_context_inner(
    ast: &FileAst,
    context: CursorContext,
) -> Vec<CompletionCandidate> {
    match &context {
        CursorContext::Use => Vec::new(), // handled by path.rs in the LSP layer
        CursorContext::Root => root_completions(ast),
        CursorContext::BindBody(scope)
        | CursorContext::InsideIf(scope)
        | CursorContext::InsideLoop(scope) => bind_body_completions(ast, scope, &context),
        CursorContext::InsideWhen(scope) => when_completions(ast, scope),
    }
}

fn root_completions(ast: &FileAst) -> Vec<CompletionCandidate> {
    let mut items = Vec::new();

    // Tags (type definitions)
    for (name, decl) in &ast.tags {
        let detail = decl
            .params
            .as_ref()
            .map(|p| format!("tag {}{}", name, format_params(p)));
        let documentation = decl.doc_comment.as_ref().map(|dc| dc.value.clone());
        items.push(CompletionCandidate {
            label: name.to_string(),
            kind: CompletionKind::Tag,
            detail,
            documentation,
            insert_text: None,
            is_snippet: false,
        });
    }

    // Defs (binds/functions)
    for (name, bind) in &ast.defs {
        let is_fn = bind.params.is_some();
        let mut detail = bind
            .params
            .as_ref()
            .map(|p| format!("{}{}", name.as_str(), format_params(p)));
        if let Some(complexity) = bind.attributes.complexity.as_ref() {
            let complexity_str = format!("complexity = {}", complexity.display_big_o());
            detail = Some(match detail {
                Some(d) => format!("{}\n{}", d, complexity_str),
                None => complexity_str,
            });
        }
        let documentation = bind.doc_comment.as_ref().map(|dc| dc.value.clone());
        items.push(CompletionCandidate {
            label: name.as_str().to_string(),
            kind: if is_fn {
                CompletionKind::Function
            } else {
                CompletionKind::Variable
            },
            detail,
            documentation,
            insert_text: None,
            is_snippet: false,
        });
    }

    // Root-level keyword: only `use` (no `tag`, no flow-control keywords)
    items.push(CompletionCandidate::new("use", CompletionKind::Keyword));

    items
}

fn bind_body_completions(
    ast: &FileAst,
    scope: &ScopeInfo,
    context: &CursorContext,
) -> Vec<CompletionCandidate> {
    let mut items: Vec<CompletionCandidate> = Vec::new();

    // 1. Keywords
    let mut keywords: Vec<(&str, Option<&str>)> = vec![
        ("if", None),
        ("for", None),
        ("while", None),
        ("in", None),
        ("when", None),
        ("return", None),
    ];

    // Snippet versions replace plain keyword for for/while/if.
    // We add both: the snippet icon will show just the keyword label.
    // The non-snippet version is for users who already typed part of it.
    if matches!(
        context,
        CursorContext::BindBody(_) | CursorContext::InsideLoop(_)
    ) {
        keywords.push(("return", None));
    }

    for (label, _) in &keywords {
        match *label {
            "for" => {
                items.push(CompletionCandidate::snippet(
                    "for",
                    "for ${1:} in ${2:}\nloop",
                    CompletionKind::Keyword,
                ));
            }
            "while" => {
                items.push(CompletionCandidate::snippet(
                    "while",
                    "while ${1:}\nloop",
                    CompletionKind::Keyword,
                ));
            }
            "if" => {
                items.push(CompletionCandidate::snippet(
                    "if",
                    "if ${1:} is ${2:}\nreturn",
                    CompletionKind::Keyword,
                ));
            }
            _ => {
                items.push(CompletionCandidate::new(label, CompletionKind::Keyword));
            }
        }
    }

    // 2. Scope variables (params first, then locals)
    for name in &scope.param_names {
        items.push(CompletionCandidate {
            label: name.clone(),
            kind: CompletionKind::Variable,
            detail: Some("parameter".to_string()),
            documentation: None,
            insert_text: None,
            is_snippet: false,
        });
    }
    for name in &scope.local_var_names {
        items.push(CompletionCandidate {
            label: name.clone(),
            kind: CompletionKind::Variable,
            detail: None,
            documentation: None,
            insert_text: None,
            is_snippet: false,
        });
    }

    // 3. Root scope symbols (tags + defs from current file)
    for (name, decl) in &ast.tags {
        let detail = decl
            .params
            .as_ref()
            .map(|p| format!("tag {}{}", name, format_params(p)));
        items.push(CompletionCandidate {
            label: name.to_string(),
            kind: CompletionKind::Tag,
            detail,
            documentation: None,
            insert_text: None,
            is_snippet: false,
        });
    }
    for (name, bind) in &ast.defs {
        let is_fn = bind.params.is_some();
        let detail = bind
            .params
            .as_ref()
            .map(|p| format!("{}{}", name.as_str(), format_params(p)));
        items.push(CompletionCandidate {
            label: name.as_str().to_string(),
            kind: if is_fn {
                CompletionKind::Function
            } else {
                CompletionKind::Variable
            },
            detail,
            documentation: None,
            insert_text: None,
            is_snippet: false,
        });
    }

    items
}

fn when_completions(ast: &FileAst, scope: &ScopeInfo) -> Vec<CompletionCandidate> {
    let mut items = bind_body_completions(ast, scope, &CursorContext::BindBody(scope.clone()));

    // Add `then` and `else` keywords specific to `when` expressions.
    items.push(CompletionCandidate::new("then", CompletionKind::Keyword));
    items.push(CompletionCandidate::new("else", CompletionKind::Keyword));

    items
}

/// Check if the cursor is after a `.` following a type name, and if that type
/// is a known union with variants, return the variant completions.
fn dot_completions_for_source(
    source: &str,
    byte_pos: usize,
    tag_types: Option<&HashMap<Intern<String>, Ty>>,
) -> Option<Vec<CompletionCandidate>> {
    let tag_types = tag_types?;

    // Must be at or after a `.`
    let line_start = source[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prefix = &source[line_start..byte_pos];

    // Find the last `.` and extract the text before it
    let dot_pos = prefix.rfind('.')?;
    let before_dot = &prefix[..dot_pos];

    // Extract the last word (the type name) — handles cases like `x: Color.`
    let last_word = before_dot.split_whitespace().last()?;

    // Must be a capitalized identifier (type name)
    if last_word.is_empty() || !last_word.chars().next()?.is_ascii_uppercase() {
        return None;
    }

    let type_name = Intern::new(last_word.to_string());
    let ty = tag_types.get(&type_name)?;

    let items = dot_completions_for_ty(ty.clone());
    if items.is_empty() {
        return None;
    }
    Some(items)
}

/// Completion labels for a literal-only union (`'debug' | 'info'`).
pub fn literal_union_completions(ty: &Ty) -> Vec<CompletionCandidate> {
    let Ty::Union { name, .. } = ty else {
        return Vec::new();
    };
    let Some(values) = ty.union_literal_values() else {
        return Vec::new();
    };
    let qualifier = name.as_str();
    values
        .iter()
        .filter_map(|v| match v {
            ConstValue::String(s) => {
                let label = format!("'{s}'");
                Some(CompletionCandidate {
                    label: label.clone(),
                    kind: CompletionKind::Keyword,
                    detail: Some(format!("{qualifier}::{label}")),
                    documentation: None,
                    insert_text: None,
                    is_snippet: false,
                })
            }
            ConstValue::Int(n) => Some(CompletionCandidate {
                label: n.to_string(),
                kind: CompletionKind::Keyword,
                detail: Some(format!("{qualifier}::{n}")),
                documentation: None,
                insert_text: None,
                is_snippet: false,
            }),
            ConstValue::Float(HashFloat(f)) => Some(CompletionCandidate {
                label: f.to_string(),
                kind: CompletionKind::Keyword,
                detail: Some(format!("{qualifier}::{f}")),
                documentation: None,
                insert_text: None,
                is_snippet: false,
            }),
            _ => None,
        })
        .collect()
}

/// Member completions after `.` when the left-hand type is a union or const union.
pub fn dot_completions_for_ty(ty: Ty) -> Vec<CompletionCandidate> {
    if ty.union_literal_values().is_some() {
        return literal_union_completions(&ty);
    }
    let Ty::Union { name, variants, .. } = &ty else {
        return Vec::new();
    };
    let qualifier = name.as_str();
    variants
        .iter()
        .map(|v| {
            let label = if v.fields.is_empty() {
                v.name.to_string()
            } else {
                let names: Vec<String> = v.fields.iter().map(|(n, _)| n.to_string()).collect();
                format!("{}({})", v.name, names.join(", "))
            };
            CompletionCandidate {
                label: label.clone(),
                kind: CompletionKind::Keyword,
                detail: Some(format!("{qualifier}.{label}")),
                documentation: None,
                insert_text: None,
                is_snippet: false,
            }
        })
        .collect()
}

/// If `byte_pos` sits in a value position after `name TypeName:`, return `TypeName`.
pub fn expected_type_name_at(source: &str, byte_pos: usize) -> Option<Intern<String>> {
    let line_start = source[..byte_pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = source[byte_pos..]
        .find('\n')
        .map(|i| byte_pos + i)
        .unwrap_or(source.len());
    let line = &source[line_start..line_end];
    let col = byte_pos.saturating_sub(line_start);
    let prefix = line.get(..col.min(line.len()))?;

    if let Some(type_name) = type_name_after_colon(prefix) {
        return Some(Intern::new(type_name.to_string()));
    }
    type_name_at_bind_site(prefix).map(|s| Intern::new(s.to_string()))
}

fn is_capitalized_type_ident(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn type_name_after_colon(prefix: &str) -> Option<&str> {
    let colon_idx = prefix.rfind(':')?;
    let before_colon = prefix[..colon_idx].trim_end();
    let after_colon = prefix[colon_idx + 1..].trim_start();
    if !after_colon.is_empty() {
        let first = after_colon.chars().next()?;
        if first != '\'' && first != '"' && !first.is_ascii_digit() && first != '-' {
            return None;
        }
    }
    let mut parts: Vec<&str> = before_colon.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let type_name = parts.pop()?;
    is_capitalized_type_ident(type_name).then_some(type_name)
}

/// `arch Architecture` at end of line — value not written yet.
fn type_name_at_bind_site(prefix: &str) -> Option<&str> {
    if prefix.contains(':') {
        return None;
    }
    let trimmed = prefix.trim_end();
    let mut parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let type_name = parts.pop()?;
    is_capitalized_type_ident(type_name).then_some(type_name)
}

/// Literal-variant completions when the cursor is typing a value for a const union type.
pub fn literal_union_completions_at(
    source: &str,
    byte_pos: usize,
    tag_types: &HashMap<Intern<String>, Ty>,
) -> Vec<CompletionCandidate> {
    let Some(type_name) = expected_type_name_at(source, byte_pos) else {
        return Vec::new();
    };
    tag_types
        .get(&type_name)
        .map(literal_union_completions)
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct SignatureInfo {
    pub label: String,
    pub params: Vec<String>,
    pub documentation: Option<String>,
}

pub fn signature_for_fn(bind: &Bind) -> Option<SignatureInfo> {
    let params = bind.params.as_ref()?;
    let label = if params.is_empty() {
        format!("{}()", bind.name.as_str())
    } else {
        format!("{}{}", bind.name.as_str(), format_params(params))
    };
    let param_list: Vec<String> = params
        .iter()
        .map(|(name, _kind)| name.to_string())
        .collect();
    Some(SignatureInfo {
        label,
        params: param_list,
        documentation: bind.doc_comment.as_ref().map(|dc| dc.value.clone()),
    })
}

pub fn fn_call_at(ast: &FileAst, byte_pos: usize) -> Option<(&Bind, SpanId)> {
    if let Some((expr, _span_id)) = ast.expr_at_byte(byte_pos)
        && let Expr::FnCall(call) = expr
    {
        // Look for a fn definition by stripping module prefixes.
        // This is parse-only; no type information available.
        return find_call_in_expr(ast, call, byte_pos);
    }
    None
}

fn find_call_in_type_surface<'a>(
    ast: &'a FileAst,
    call_path: &ModPath,
) -> Option<(&'a Bind, SpanId)> {
    // The function name is the root if no segments, or the last segment.
    let fn_name = if call_path.segments.is_empty() {
        &call_path.root
    } else {
        call_path.segments.last()?
    };

    if let Some(bind) = ast.defs.get(fn_name) {
        return bind.params.as_ref().map(|_| (bind, bind.name_span));
    }
    None
}

fn find_call_in_expr<'a>(
    ast: &'a FileAst,
    call: &ast::FnCall,
    _byte_pos: usize,
) -> Option<(&'a Bind, SpanId)> {
    find_call_in_type_surface(ast, &call.path.value)
}

pub fn format_params(params: &Parameters) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|(name, parameter)| {
            if let ParameterKind::Tagged(sp) = &parameter.kind
                && let Expr::AnonymousTag(type_name) = &sp.value
                && name.eq_ignore_ascii_case(type_name.as_str())
            {
                let short = name.chars().next().unwrap_or('p');
                return format!("{short} {}", sp.value.format_surface());
            }
            match &parameter.kind {
                ParameterKind::Generic => name.to_string(),
                ParameterKind::Tagged(ty) => {
                    format!("{name} {}", ty.value.format_surface())
                }
                ParameterKind::ValueParam { ty } => {
                    format!("{name} {}", ty.value.format_surface())
                }
                ParameterKind::Inferred { ty } => {
                    format!("{name} {}: ?", ty.value.format_surface())
                }
                ParameterKind::Default(expr) => match &expr.value {
                    Expr::Lit(literal) => format!("{name}: {literal}"),
                    _ => format!("{name}: {expr:?}"),
                },
            }
        })
        .collect();
    format!("({})", parts.join(", "))
}

#[cfg(test)]
#[path = "../tests/completions_tests.rs"]
mod tests;
