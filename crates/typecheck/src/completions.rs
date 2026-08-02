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
                        for child in [condition.as_ref(), body.as_ref()] {
                            if st.contains(child.span_id, byte_pos) {
                                return detect_block_in_expr(st, &child.value, byte_pos);
                            }
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
    let Ty::Union { name, variants, .. } = ty else {
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

#[allow(dead_code)]
fn find_call_in_bind_value<'a>(
    ast: &'a FileAst,
    val: &'a BindValue,
    byte_pos: usize,
) -> Option<(&'a Bind, SpanId)> {
    match val {
        BindValue::Expr(expr) => {
            if let Expr::FnCall(call) = &expr.value
                && ast.span_table.contains(call.path.span_id, byte_pos)
            {
                return find_call_in_expr(ast, call, byte_pos);
            }
            None
        }
        _ => None,
    }
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
mod tests {
    use super::*;
    use crate::ty::UnionVariant;
    use ast::span::SpanId;
    use ast::{Literal, Parameter, Spanned, Typed};
    use indexmap::IndexMap;
    use internment::Intern;
    use std::collections::HashMap;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    fn make_params(items: Vec<(Intern<String>, ParameterKind)>) -> Parameters {
        let mut map = IndexMap::new();
        for (name, kind) in items {
            map.insert(name, Parameter::new(SpanId::new(0), kind));
        }
        map
    }

    fn tagged_param(name: &str, type_name: &str) -> (Intern<String>, ParameterKind) {
        (
            intern(name),
            ParameterKind::Tagged(Box::new(Spanned {
                value: Expr::AnonymousTag(intern(type_name)),
                span_id: SpanId::new(0),
            })),
        )
    }

    #[test]
    fn format_params_shortens_param_matching_type_name() {
        let params = make_params(vec![tagged_param("str", "Str")]);
        assert_eq!(format_params(&params), "(s Str)");
    }

    #[test]
    fn format_params_does_not_shorten_different_name() {
        let params = make_params(vec![tagged_param("string", "Str")]);
        assert_eq!(format_params(&params), "(string Str)");
    }

    #[test]
    fn format_params_shortens_case_insensitive_match() {
        let params = make_params(vec![tagged_param("STR", "Str")]);
        assert_eq!(format_params(&params), "(S Str)");
    }

    #[test]
    fn expected_type_name_at_colon_value() {
        let src = "    level LogLevel: '";
        let pos = src.len() - 1;
        assert_eq!(
            expected_type_name_at(src, pos).map(|n| n.as_str().to_string()),
            Some("LogLevel".to_string())
        );
    }

    #[test]
    fn literal_union_completions_for_log_level() {
        let src = "level LogLevel: '";
        let ty = Ty::union_of_literals(
            Intern::new("LogLevel".to_string()),
            vec![
                ConstValue::String("debug".to_string()),
                ConstValue::String("info".to_string()),
            ],
        );
        let mut tag_types = HashMap::new();
        tag_types.insert(Intern::new("LogLevel".to_string()), ty);
        let items = literal_union_completions_at(src, src.len() - 1, &tag_types);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.label == "'debug'"));
        assert!(items.iter().any(|i| i.label == "'info'"));
    }

    #[test]
    fn format_params_empty_params() {
        let params: Parameters = IndexMap::new();
        assert_eq!(format_params(&params), "");
    }

    #[test]
    fn format_params_multiple_mixed() {
        let params = make_params(vec![
            tagged_param("str", "Str"),
            tagged_param("count", "Int"),
        ]);
        assert_eq!(format_params(&params), "(s Str, count Int)");
    }

    #[test]
    fn format_params_generic() {
        let params = make_params(vec![(intern("T"), ParameterKind::Generic)]);
        assert_eq!(format_params(&params), "(T)");
    }

    #[test]
    fn format_params_default() {
        let expr = Expr::Lit(Literal::Number(42));
        let params = make_params(vec![(
            intern("x"),
            ParameterKind::Default(Box::new(Typed::infer(expr, SpanId::new(0)))),
        )]);
        assert_eq!(format_params(&params), "(x: 42)");
    }

    #[test]
    fn test_detect_root_context() {
        let ast = FileAst::empty_for_tests();
        let src = "\n\n";
        let ctx = detect_cursor_context(&ast, src, 1);
        assert_eq!(ctx, CursorContext::Root);

        // On a `use` line
        let src = "use core\n";
        let ctx = detect_cursor_context(&ast, src, 4);
        assert_eq!(ctx, CursorContext::Use);
    }

    #[test]
    fn test_detect_context_use_line() {
        let ast = FileAst::empty_for_tests();
        let src = "use core.Bool\n";
        let ctx = detect_cursor_context(&ast, src, 5);
        assert_eq!(ctx, CursorContext::Use);
    }

    #[test]
    fn test_root_completions_only_use_keyword() {
        let ast = FileAst::empty_for_tests();
        let items = root_completions(&ast);
        let keywords: Vec<&str> = items
            .iter()
            .filter(|c| matches!(c.kind, CompletionKind::Keyword))
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(keywords, vec!["use"]);
    }

    #[test]
    fn test_dot_completions_for_source_no_match() {
        let src = "x := 42";
        let result = dot_completions_for_source(src, src.len(), None);
        assert!(result.is_none());
    }

    #[test]
    fn test_new_completion_candidate() {
        let c = CompletionCandidate::new("foo", CompletionKind::Keyword);
        assert_eq!(c.label, "foo");
        assert_eq!(c.insert_text, None);
        assert!(!c.is_snippet);
    }

    #[test]
    fn test_snippet_completion_candidate() {
        let c = CompletionCandidate::snippet(
            "for",
            "for ${1:} in ${2:}\nloop",
            CompletionKind::Keyword,
        );
        assert_eq!(c.label, "for");
        assert_eq!(c.insert_text, Some("for ${1:} in ${2:}\nloop".to_string()));
        assert!(c.is_snippet);
    }

    #[test]
    fn root_completions_shows_tags() {
        // In Gin, tag declarations use a capitalized name directly (no `tag` keyword).
        let src = "LogLevel is 'debug' | 'info'";
        let ast = parser::parse_from_str(src);
        assert!(
            ast.tags.contains_key(&Intern::new("LogLevel".to_string())),
            "LogLevel should be parsed as a tag"
        );
        let items = root_completions(&ast);
        assert!(
            items
                .iter()
                .any(|c| c.label == "LogLevel" && matches!(c.kind, CompletionKind::Tag)),
            "root_completions should include the LogLevel tag"
        );
    }

    #[test]
    fn root_completions_shows_defs() {
        let src = r#"
two := 1 + 1

fn inc(x Int) Int := x + 1
"#;
        let ast = parser::parse_from_str(src);
        let items = root_completions(&ast);

        // 'two' is a variable (no params)
        assert!(
            items
                .iter()
                .any(|c| c.label == "two" && matches!(c.kind, CompletionKind::Variable)),
            "root_completions should include 'two' as Variable"
        );

        // 'inc' is a function (has params)
        assert!(
            items
                .iter()
                .any(|c| c.label == "inc" && matches!(c.kind, CompletionKind::Function)),
            "root_completions should include 'inc' as Function"
        );
    }

    #[test]
    fn root_completions_keyword_use() {
        // Verify `use` keyword is last among keywords.
        let ast = parser::parse_from_str("");
        let items = root_completions(&ast);
        let keywords: Vec<&str> = items
            .iter()
            .filter(|c| matches!(c.kind, CompletionKind::Keyword))
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(keywords, vec!["use"]);
    }

    #[test]
    fn bind_body_completions_keywords() {
        let src = r#"
main:
    return 0
"#;
        let ast = parser::parse_from_str(src);
        let scope = collect_scope_info(&ast, src.len() - 3);
        let items =
            bind_body_completions(&ast, &scope, &CursorContext::BindBody(ScopeInfo::default()));

        let keyword_labels: Vec<&str> = items
            .iter()
            .filter(|c| matches!(c.kind, CompletionKind::Keyword))
            .map(|c| c.label.as_str())
            .collect();

        for kw in &["if", "for", "while", "when", "in", "return"] {
            assert!(
                keyword_labels.contains(kw),
                "bind body should have keyword '{}'",
                kw
            );
        }

        // 'for', 'while', 'if' should be snippets
        for snippet_label in &["for", "while", "if"] {
            let item = items.iter().find(|c| c.label == *snippet_label).unwrap();
            assert!(item.is_snippet, "'{}' should be a snippet", snippet_label);
            assert!(
                item.insert_text.is_some(),
                "'{}' snippet should have insert_text",
                snippet_label
            );
        }
    }

    #[test]
    fn bind_body_completions_shows_params() {
        let src = r#"
greet(name Str, count Int) Str:
    return name
"#;
        let ast = parser::parse_from_str(src);
        // Position inside the body after the return keyword
        let scope = collect_scope_info(&ast, src.len() - 2);

        let param_names: Vec<&str> = scope.param_names.iter().map(|s| s.as_str()).collect();
        assert!(
            param_names.contains(&"name"),
            "scope should include 'name' param"
        );
        assert!(
            param_names.contains(&"count"),
            "scope should include 'count' param"
        );
    }

    #[test]
    fn bind_body_completions_shows_locals() {
        let src = r#"
main:
    x := 1
    y: 2
    return 0
"#;
        let ast = parser::parse_from_str(src);
        // Use a position inside `return 0` to ensure we're past both locals
        let pos = src.rfind("return").unwrap_or(src.len() - 4);
        let scope = collect_scope_info(&ast, pos);
        assert!(
            scope.local_var_names.iter().any(|s| s == "x"),
            "scope should include local 'x', got {:?}",
            scope.local_var_names
        );
        assert!(
            scope.local_var_names.iter().any(|s| s == "y"),
            "scope should include local 'y', got {:?}",
            scope.local_var_names
        );
    }

    #[test]
    fn bind_body_completions_shows_scope_tags_and_defs() {
        let src = r#"
Level is 'low' | 'high'

check(l Level) Int:
    return 0
"#;
        let ast = parser::parse_from_str(src);
        // Use a position on the return value (within the ret span)
        let scope = collect_scope_info(&ast, src.len() - 2);
        let items = bind_body_completions(&ast, &scope, &CursorContext::BindBody(scope.clone()));

        // Should include the tag 'Level'
        assert!(
            items
                .iter()
                .any(|c| c.label == "Level" && matches!(c.kind, CompletionKind::Tag)),
            "body completions should show tag 'Level'"
        );

        // Should include the def 'check'
        assert!(
            items
                .iter()
                .any(|c| c.label == "check" && matches!(c.kind, CompletionKind::Function)),
            "body completions should show function 'check'"
        );

        // Should include the param 'l'
        assert!(
            items
                .iter()
                .any(|c| c.label == "l" && matches!(c.kind, CompletionKind::Variable)),
            "body completions should show param 'l'"
        );
    }

    #[test]
    fn when_completions_shows_then_else() {
        let src = r#"
main:
    when x

"#;
        let ast = parser::parse_from_str(src);
        let scope = collect_scope_info(&ast, src.len());
        let items = when_completions(&ast, &scope);

        assert!(
            items
                .iter()
                .any(|c| c.label == "then" && matches!(c.kind, CompletionKind::Keyword)),
            "when completions should include 'then'"
        );
        assert!(
            items
                .iter()
                .any(|c| c.label == "else" && matches!(c.kind, CompletionKind::Keyword)),
            "when completions should include 'else'"
        );
    }

    #[test]
    fn dot_completions_for_ty_union_variants() {
        let ty = Ty::union_named(
            Intern::new("Color".to_string()),
            vec![
                UnionVariant::new(Intern::new("Red".to_string()), vec![]),
                UnionVariant::new(Intern::new("Green".to_string()), vec![]),
                UnionVariant::new(Intern::new("Blue".to_string()), vec![]),
            ],
        );
        let items = dot_completions_for_ty(ty);

        assert_eq!(items.len(), 3);
        for expected in &["Red", "Green", "Blue"] {
            assert!(
                items.iter().any(|c| c.label == *expected),
                "dot completions should include '{}'",
                expected
            );
        }
    }

    #[test]
    fn dot_completions_for_ty_literal_union() {
        let ty = Ty::union_of_literals(
            Intern::new("LogLevel".to_string()),
            vec![
                ConstValue::String("debug".to_string()),
                ConstValue::String("info".to_string()),
            ],
        );
        let items = dot_completions_for_ty(ty);

        assert_eq!(items.len(), 2);
        assert!(
            items.iter().any(|c| c.label == "'debug'"),
            "literal union dot completions should include ''debug''"
        );
        assert!(
            items.iter().any(|c| c.label == "'info'"),
            "literal union dot completions should include ''info''"
        );
    }

    #[test]
    fn dot_completions_for_source_match() {
        let src = "color: Color.";
        let ty = Ty::union_named(
            Intern::new("Color".to_string()),
            vec![
                UnionVariant::new(Intern::new("Red".to_string()), vec![]),
                UnionVariant::new(Intern::new("Blue".to_string()), vec![]),
            ],
        );
        let mut tag_types = HashMap::new();
        tag_types.insert(Intern::new("Color".to_string()), ty);

        let result = dot_completions_for_source(src, src.len(), Some(&tag_types));
        assert!(result.is_some(), "should return completions for Color.");

        let items = result.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|c| c.label == "Red"));
        assert!(items.iter().any(|c| c.label == "Blue"));
    }

    #[test]
    fn dot_completions_for_source_no_match_non_type() {
        // lowercase before dot should not match
        let src = "x: value.";
        let result = dot_completions_for_source(src, src.len(), None);
        assert!(result.is_none(), "lowercase before dot should not match");
    }

    #[test]
    fn detect_cursor_context_bind_body() {
        let src = "main:\n    return 0\n";
        let ast = parser::parse_from_str(src);
        // Cursor on the indented 'return'
        let ctx = detect_cursor_context(&ast, src, 10);
        assert!(
            matches!(ctx, CursorContext::BindBody(_)),
            "indented line should be BindBody, got {:?}",
            ctx
        );
    }

    #[test]
    fn detect_cursor_context_inside_if() {
        let src = r#"
main:
    if x is 1
        return y
"#;
        let ast = parser::parse_from_str(src);
        // Position on the `if` keyword itself
        let pos = src.find("if x").unwrap_or(0);
        let ctx = detect_cursor_context(&ast, src, pos);
        // Inside the if expression or bind body are both acceptable
        assert!(
            matches!(ctx, CursorContext::InsideIf(_) | CursorContext::BindBody(_)),
            "cursor at 'if' should be InsideIf or BindBody, got {:?}",
            ctx
        );
    }

    #[test]
    fn detect_cursor_context_inside_when() {
        let src = r#"
main:
    when x
        then return 1
"#;
        let ast = parser::parse_from_str(src);
        // Cursor on 'then' inside the when body
        let pos = src.find('t').unwrap_or(0);
        let ctx = detect_cursor_context(&ast, src, pos);
        assert!(
            matches!(ctx, CursorContext::InsideWhen(_)),
            "cursor inside when body should be InsideWhen, got {:?}",
            ctx
        );
    }

    #[test]
    fn detect_cursor_context_inside_loop() {
        let src = r#"
main:
    for x in items
        loop
"#;
        let ast = parser::parse_from_str(src);
        // Cursor on 'loop' keyword inside the for body
        let pos = src.rfind("loop").unwrap_or(src.len() - 2);
        let ctx = detect_cursor_context(&ast, src, pos);
        assert!(
            matches!(ctx, CursorContext::InsideLoop(_)),
            "cursor inside loop body should be InsideLoop, got {:?}",
            ctx
        );
    }

    #[test]
    fn collect_scope_info_locals_before_cursor() {
        let src = r#"
main:
    a := 1
    b := 2
    return 0
"#;
        let ast = parser::parse_from_str(src);

        // Position after both a and b, inside return
        let pos_after = src.rfind("return").unwrap_or(src.len() - 4);
        let scope_after = collect_scope_info(&ast, pos_after);
        assert!(
            scope_after.local_var_names.iter().any(|s| s == "a"),
            "after both decls, 'a' should be visible, got {:?}",
            scope_after.local_var_names
        );
        assert!(
            scope_after.local_var_names.iter().any(|s| s == "b"),
            "after both decls, 'b' should be visible, got {:?}",
            scope_after.local_var_names
        );

        // Position before 'b' is declared
        let pos_before_b = src.find("a := 1").unwrap_or(0);
        let scope_before = collect_scope_info(&ast, pos_before_b);
        assert!(
            !scope_before.local_var_names.iter().any(|s| s == "b"),
            "before 'b', 'b' should NOT be visible, got {:?}",
            scope_before.local_var_names
        );
    }

    #[test]
    fn expected_type_name_at_bind_site() {
        let src = "x Int";
        let pos = src.len();
        let result = expected_type_name_at(src, pos);
        assert_eq!(
            result.map(|n| n.as_str().to_string()),
            Some("Int".to_string())
        );
    }

    #[test]
    fn expected_type_name_at_with_existing_value() {
        let src = "x String: 'he";
        let pos = src.len();
        let result = expected_type_name_at(src, pos);
        assert_eq!(
            result.map(|n| n.as_str().to_string()),
            Some("String".to_string())
        );
    }

    #[test]
    fn expected_type_name_at_returns_none_for_lowercase() {
        let src = "x string";
        let pos = src.len();
        let result = expected_type_name_at(src, pos);
        assert_eq!(result, None, "lowercase type name should return None");
    }

    #[test]
    fn signature_for_fn_with_params() {
        let src = r#"
add(a Int, b Int) Int:
    return a + b
"#;
        let ast = parser::parse_from_str(src);
        let bind = ast
            .defs
            .get(&Intern::new("add".to_string()))
            .expect("add function should exist");
        let info = signature_for_fn(bind).expect("should produce signature info");

        assert!(info.label.contains("add"));
        assert_eq!(info.params.len(), 2);
        assert!(info.params.contains(&"a".to_string()));
        assert!(info.params.contains(&"b".to_string()));
    }

    #[test]
    fn signature_for_fn_no_params() {
        let src = r#"
constant := 42
"#;
        let ast = parser::parse_from_str(src);
        let bind = ast
            .defs
            .get(&Intern::new("constant".to_string()))
            .expect("constant bind should exist");
        // No params → signature_for_fn should return None
        assert!(signature_for_fn(bind).is_none());
    }

    #[test]
    fn completions_for_context_root() {
        let src = r#"
Greeting is 'hello' | 'goodbye'
"#;
        let ast = parser::parse_from_str(src);
        // Cursor at root level (no indent)
        let items = ast.completions_for_context(src, 1, None);
        assert!(
            items
                .iter()
                .any(|c| c.label == "Greeting" && matches!(c.kind, CompletionKind::Tag)),
            "root context should include 'Greeting' tag"
        );
        assert!(
            items
                .iter()
                .any(|c| c.label == "use" && matches!(c.kind, CompletionKind::Keyword)),
            "root context should include 'use' keyword"
        );
    }

    #[test]
    fn completions_for_context_literal_union() {
        let src = "level LogLevel: '";
        let ty = Ty::union_of_literals(
            Intern::new("LogLevel".to_string()),
            vec![
                ConstValue::String("debug".to_string()),
                ConstValue::String("info".to_string()),
            ],
        );
        let mut tag_types = HashMap::new();
        tag_types.insert(Intern::new("LogLevel".to_string()), ty);

        let ast = FileAst::empty_for_tests();
        let items = ast.completions_for_context(src, src.len(), Some(&tag_types));
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.label == "'debug'"));
        assert!(items.iter().any(|i| i.label == "'info'"));
    }

    #[test]
    fn completions_for_context_dot_completion() {
        let src = "color: Color.";
        let ty = Ty::union_named(
            Intern::new("Color".to_string()),
            vec![
                UnionVariant::new(Intern::new("Red".to_string()), vec![]),
                UnionVariant::new(Intern::new("Blue".to_string()), vec![]),
            ],
        );
        let mut tag_types = HashMap::new();
        tag_types.insert(Intern::new("Color".to_string()), ty);

        let ast = FileAst::empty_for_tests();
        let items = ast.completions_for_context(src, src.len(), Some(&tag_types));
        assert!(!items.is_empty(), "dot completions should not be empty");
        assert!(items.iter().any(|i| i.label == "Red"));
        assert!(items.iter().any(|i| i.label == "Blue"));
    }

    #[test]
    fn completions_for_context_bind_body() {
        let src = r#"
main:
    return 0
"#;
        let ast = parser::parse_from_str(src);
        // Cursor on the return expression
        let pos = src.find("return").unwrap_or(src.len() - 4);
        let items = ast.completions_for_context(src, pos, None);
        assert!(
            items.iter().any(|c| c.label == "return"),
            "bind body context should include 'return' keyword"
        );
        assert!(
            items.iter().any(|c| c.label == "if"),
            "bind body context should include 'if' keyword"
        );
    }
}
