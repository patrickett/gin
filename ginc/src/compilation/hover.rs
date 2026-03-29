//! Hover information generation for the LSP.

use crate::ast::{
    Bind, BindValue, Declare, DeclareValue, Expr, FileAst, IfCondition, Literal, ParameterKind,
    Spanned, Tag, Variant,
};
use crate::database::File;
use crate::database::input_database::Db;
use crate::typeck::{Ty, TyEnv, ty_alignment, ty_byte_size_static};

// ── Trait & context ──────────────────────────────────────────────────────────

/// Shared context available to every `HoverDoc` implementation.
pub struct HoverContext<'a> {
    pub module: &'a str,
    pub ast: &'a FileAst,
    pub ty_env: &'a TyEnv,
}

/// Produce a markdown hover string for an item.
/// Returns `None` when this item has no hover representation (e.g. an unknown keyword).
pub trait HoverDoc {
    fn hover_doc(&self, ctx: &HoverContext<'_>) -> Option<String>;
}

// ── Wrapper types ─────────────────────────────────────────────────────────────

/// A `Bind` in local (body) scope with an optional flow-narrowed type.
pub struct LocalBind<'a> {
    pub bind: &'a Bind,
    pub narrowed_type: Option<String>,
}

/// A gin language keyword.
pub struct Keyword<'a>(pub &'a str);

/// The `self` reference inside a method body.
pub struct SelfRef<'a> {
    pub receiver_name: &'a str,
    pub decl: Option<&'a Declare>,
}

// ── HoverDoc implementations ─────────────────────────────────────────────────

impl HoverDoc for Bind {
    fn hover_doc(&self, ctx: &HoverContext<'_>) -> Option<String> {
        let name = if let Some(recv) = self.receiver_type() {
            format!("{}.{}", recv.name(), self.name().as_str())
        } else {
            self.name().as_str().to_string()
        };
        let mut md = format!("*{}*\n\n```gin\n{name}", ctx.module);
        if let Some(params) = self.params() {
            md.push_str(&format_params(params));
        }
        let ret_type = compute_rich_return_type(self, ctx.ast)
            .or_else(|| self.infer_return_type_union_with_defs(ctx.ast.defs()));
        if let Some(ret) = ret_type {
            md.push_str(&format!(" {ret}"));
        }
        md.push_str("\n```\n");
        if let Some(doc) = self.doc_comment() {
            md.push_str("\n---\n\n");
            md.push_str(&doc.0);
        }
        Some(md)
    }
}

impl HoverDoc for LocalBind<'_> {
    fn hover_doc(&self, ctx: &HoverContext<'_>) -> Option<String> {
        let bind = self.bind;
        let mut md = format!("```gin\n{}", bind.name().as_str());
        if let Some(params) = bind.params() {
            md.push_str(&format_params(params));
        }
        if let Some(narrowed) = &self.narrowed_type {
            md.push_str(&format!(" {narrowed}"));
        } else if let Some((type_name, args)) = bind.type_annotation.as_ref() {
            let args_str: Vec<String> = args.iter().map(|a| display_lit_expr(a)).collect();
            md.push_str(&format!(" {}({})", type_name.as_str(), args_str.join(", ")));
        } else if let Some(inferred) = infer_type_from_bind_with_ast(bind, ctx.ast) {
            md.push_str(&format!(" {inferred}"));
        } else if let Some(inferred_type) = bind.infer_return_type() {
            md.push_str(&format!(" {inferred_type}"));
        }
        md.push_str("\n```\n");
        if let Some(doc) = bind.doc_comment() {
            md.push_str("\n---\n\n");
            md.push_str(&doc.0);
        }
        Some(md)
    }
}

impl HoverDoc for Declare {
    fn hover_doc(&self, ctx: &HoverContext<'_>) -> Option<String> {
        let ty = ctx.ty_env.resolve_tag(&Tag::Nominal(
            self.name(),
            crate::prelude::SimpleSpan::from(0..0),
        ));
        let block = format_declare(self);
        let mut md = format!("*{}*\n\n```gin\n{block}\n```\n", ctx.module);
        if !matches!(ty, Ty::Opaque(_)) && !contains_opaque(&ty) {
            let size = ty_byte_size_static(&ty);
            let align = ty_alignment(&ty);
            md.push_str(&format!(
                "\n---\n\nsize = {size} ({size:#x}), align = {align:#x}\n"
            ));
        }
        if let Some(doc) = self.doc_comment() {
            md.push_str("\n---\n\n");
            md.push_str(&doc.0);
        }
        Some(md)
    }
}

impl HoverDoc for Variant {
    fn hover_doc(&self, ctx: &HoverContext<'_>) -> Option<String> {
        let (tag, doc) = match self {
            Variant::External(tag) => (tag, None),
            Variant::Local { tag, doc_comment } => (tag, doc_comment.as_ref()),
        };
        let mut result = format!("*{}*\n\n```gin\n{}\n```", ctx.module, tag);
        if let Some(doc) = doc {
            result.push_str("\n\n---\n\n");
            result.push_str(&doc.0);
        }
        Some(result)
    }
}

impl HoverDoc for Keyword<'_> {
    fn hover_doc(&self, _ctx: &HoverContext<'_>) -> Option<String> {
        let doc = match self.0 {
            "is" => {
                "Tag declaration keyword.\n\nDeclares a tag as a specific value or union.\n\n```gin\nColor is Red or Green or Blue\n```"
            }
            "has" => {
                "Struct field declaration keyword.\n\nDeclares that a tag has named fields.\n\n```gin\nPoint has x Float, y Float\n```"
            }
            "or" => {
                "Union variant separator.\n\nSeparates variants in a union tag declaration.\n\n```gin\nResult is Ok or Err\n```"
            }
            "return" => "Return a value from a function.",
            ":" => {
                "Binding operator.\n\nAnnotates a value with a tag or binds a default parameter value."
            }
            "if" => "Conditional expression.",
            "for" => "For-in loop.",
            "while" => "While loop.",
            "when" => "Pattern matching expression.",
            "in" => "Range or iteration operator.",
            "as" => "Type cast operator.",
            "not" => "Logical negation.",
            "use" => "Import declaration.",
            _ => return None,
        };
        Some(format!("```gin\n{}\n```\n\n---\n\n{doc}", self.0))
    }
}

impl HoverDoc for SelfRef<'_> {
    fn hover_doc(&self, _ctx: &HoverContext<'_>) -> Option<String> {
        let type_str = if let Some(d) = self.decl {
            if let Some(params) = d.params() {
                if params.is_empty() {
                    self.receiver_name.to_string()
                } else {
                    let param_names: Vec<String> = params.keys().map(|k| k.to_string()).collect();
                    format!("{}({})", self.receiver_name, param_names.join(", "))
                }
            } else {
                self.receiver_name.to_string()
            }
        } else {
            self.receiver_name.to_string()
        };
        Some(format!("```gin\nself {type_str}\n```"))
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Return markdown hover text for the word at `byte_pos` in `file`.
/// Returns `None` if there is nothing hover-able at that position.
pub fn hover_at(db: &dyn Db, file: File, byte_pos: usize) -> Option<String> {
    let source = file.contents(db);
    let path = file.path(db);
    let module = module_name_from_path(&path);
    let ast = crate::parse::parse::parse(db, file);
    let ty_env = crate::compilation::compile::shared_ty_env(db, file);
    let ctx = HoverContext {
        module: &module,
        ast: &ast,
        ty_env: &ty_env,
    };
    let word = word_at_byte_pos(source, byte_pos)?;
    hover_for_word(&word, source, byte_pos, &ctx)
}

// ── Cascade ───────────────────────────────────────────────────────────────────

fn hover_for_word(
    word: &str,
    source: &str,
    byte_pos: usize,
    ctx: &HoverContext<'_>,
) -> Option<String> {
    // Numeric literals
    if word.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("```gin\n{word}\n```"));
    }

    // `self` inside a method body
    if word == "self"
        && let Some(recv_name) = enclosing_method_receiver(source, byte_pos)
    {
        let decl = ctx
            .ast
            .tags()
            .iter()
            .find(|(k, _)| k.as_str() == recv_name)
            .map(|(_, v)| v);
        return SelfRef {
            receiver_name: recv_name,
            decl,
        }
        .hover_doc(ctx);
    }

    // Union variants
    for decl in ctx.ast.tags().values() {
        if let DeclareValue::Union { variants } = decl.value() {
            for variant in variants {
                if variant.tag().name() == word {
                    return variant.hover_doc(ctx);
                }
            }
        }
    }

    // Type declarations
    for (name, decl) in ctx.ast.tags() {
        if name.as_str() == word {
            return decl.hover_doc(ctx);
        }
    }

    // Top-level bindings
    for (name, bind) in ctx.ast.defs() {
        let name_str = name.as_str();
        let matches = name_str == word
            || (name_str.contains('.') && name_str.split('.').next_back() == Some(word));
        if matches {
            return bind.hover_doc(ctx);
        }
    }

    // Local bindings inside function bodies
    for bind in ctx.ast.defs().values() {
        if let BindValue::Body { exprs, .. } = bind.value()
            && let Some(local_bind) = find_local_bind_recursive(exprs, word)
        {
            let narrowed_type = match local_bind.value() {
                BindValue::Expr(e) => eval_expr_to_literal(e, ctx.ast).map(|v| v.to_string()),
                _ => None,
            };
            return LocalBind {
                bind: local_bind,
                narrowed_type,
            }
            .hover_doc(ctx);
        }
    }

    // Parameters
    for bind in ctx.ast.defs().values() {
        if let Some(params) = bind.params() {
            for (param_name, kind) in params {
                if param_name.as_str() == word {
                    let ty_str = match kind {
                        ParameterKind::Generic => word.to_string(),
                        ParameterKind::Tagged(tag) => format!("{word} {tag}"),
                        ParameterKind::Default(_) => word.to_string(),
                    };
                    return Some(format!("```gin\n{ty_str}\n```"));
                }
                if let ParameterKind::Tagged(tag) = kind
                    && tag.name() == word
                {
                    if let Some(decl) = ctx
                        .ast
                        .tags()
                        .get(&crate::intern::IStr::new(word.to_string()))
                    {
                        return decl.hover_doc(ctx);
                    }
                    return Some(format!("```gin\n{word}\n```"));
                }
            }
        }
    }

    // Keywords
    if let Some(result) = Keyword(word).hover_doc(ctx) {
        return Some(result);
    }

    // Pattern variables (e.g. `v` in `if val is Some(v)`)
    if let Some(inferred) = infer_pattern_var_value(word, ctx.ast) {
        return Some(format!("```gin\n{word} {inferred}\n```"));
    }

    // Generic type parameters inside tag parens
    if word
        .chars()
        .all(|c| c.is_alphabetic() && c.is_lowercase() || c == '_')
        && is_in_tag_params(source, byte_pos)
    {
        return Some(format!("```gin\n{word}\n```"));
    }

    None
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn module_name_from_path(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn word_at_byte_pos(source: &str, byte_pos: usize) -> Option<String> {
    let mut start = byte_pos;
    let mut end = byte_pos;
    let bytes = source.as_bytes();
    while start > 0 && is_identifier_char(bytes[start - 1] as char) {
        start -= 1;
    }
    while end < bytes.len() && is_identifier_char(bytes[end] as char) {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(source[start..end].to_string())
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Pretty-print a declaration for hover. Uses multiline for unions with ≥3 variants or >80 chars.
fn format_declare(decl: &Declare) -> String {
    let mut lhs = decl.name().as_str().to_string();
    if let Some(params) = decl.params() {
        lhs.push('(');
        let mut first = true;
        for (k, v) in params {
            if !first {
                lhs.push_str(", ");
            }
            first = false;
            lhs.push_str(k.as_str());
            lhs.push_str(&v.to_string());
        }
        lhs.push(')');
    }
    match decl.value() {
        DeclareValue::Union { variants } => {
            let single_line = format!("{lhs} is {}", decl.value());
            if variants.len() <= 2 && single_line.len() <= 80 {
                single_line
            } else {
                let mut lines = vec![format!("{lhs} is")];
                for (i, v) in variants.iter().enumerate() {
                    let suffix = if i < variants.len() - 1 { " or" } else { "" };
                    lines.push(format!("    {v}{suffix}"));
                }
                lines.join("\n")
            }
        }
        _ => format!("{decl}"),
    }
}

fn contains_opaque(ty: &Ty) -> bool {
    match ty {
        Ty::Opaque(_) => true,
        Ty::Record { fields, .. } => fields.iter().any(|(_, t)| contains_opaque(t)),
        Ty::Union { variants, .. } => variants
            .iter()
            .any(|(_, fields)| fields.iter().any(|(_, t)| contains_opaque(t))),
        Ty::Array { elem, .. } | Ty::Ptr { inner: elem } | Ty::Ref { inner: elem } => {
            contains_opaque(elem)
        }
        Ty::Tuple(fields) => fields.iter().any(contains_opaque),
        _ => false,
    }
}

fn display_lit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Lit(Literal::Int(n)) => n.to_string(),
        Expr::Lit(Literal::Number(n)) => n.to_string(),
        Expr::Lit(Literal::Float(f)) => f.to_string(),
        Expr::Lit(Literal::String(s)) => format!("\"{s}\""),
        _ => "_".to_string(),
    }
}

fn infer_type_from_bind_with_ast(bind: &Bind, ast: &FileAst) -> Option<String> {
    let expr = match bind.value() {
        BindValue::Expr(e) => Some(e.as_ref()),
        BindValue::Body { ret, .. } => ret.0.as_ref().map(|e| e.as_ref()),
        BindValue::Extern => return None,
    };
    let expr = expr?;

    if let Expr::TagCall(tc) = &expr.0 {
        for decl in ast.tags().values() {
            if let DeclareValue::Union { variants } = decl.value() {
                for variant in variants {
                    let tag = variant.tag();
                    if tag.name() == tc.name.as_str() {
                        let params_str = if let Some(union_params) = decl.params() {
                            if union_params.is_empty() {
                                String::new()
                            } else {
                                let variant_param_order: Vec<&str> = match tag {
                                    Tag::Generic(_, vparams, _) => {
                                        vparams.keys().map(|k| k.as_str()).collect()
                                    }
                                    _ => vec![],
                                };
                                let args: Vec<String> = union_params
                                    .keys()
                                    .map(|uparam| {
                                        let idx = variant_param_order
                                            .iter()
                                            .position(|&vp| vp == uparam.as_str());
                                        let arg_expr = idx.and_then(|i| tc.args.get(i));
                                        match arg_expr {
                                            Some(e) => match &**e {
                                                Expr::Lit(Literal::Int(n)) => n.to_string(),
                                                Expr::Lit(Literal::Number(n)) => n.to_string(),
                                                Expr::Lit(Literal::Float(f)) => f.to_string(),
                                                _ => uparam.to_string(),
                                            },
                                            None => uparam.to_string(),
                                        }
                                    })
                                    .collect();
                                format!("({})", args.join(", "))
                            }
                        } else {
                            String::new()
                        };
                        return Some(format!("{}{}", decl.name(), params_str));
                    }
                }
            }
        }
        return Some(tc.name.to_string());
    }

    None
}

fn eval_expr_to_literal(expr: &Expr, ast: &FileAst) -> Option<i128> {
    eval_expr_to_literal_with_locals(expr, ast, &[])
}

fn eval_expr_to_literal_with_locals(expr: &Expr, ast: &FileAst, locals: &[&Expr]) -> Option<i128> {
    use crate::prelude::BinOp;
    match expr {
        Expr::Lit(Literal::Int(n)) => Some(*n),
        Expr::Lit(Literal::Number(n)) => Some(*n as i128),
        Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
            let var = call.path.root.as_str();
            for local in locals {
                if let Expr::Bind(b) = local
                    && b.name().as_str() == var
                    && let BindValue::Expr(e) = b.value()
                {
                    return eval_expr_to_literal_with_locals(e, ast, locals);
                }
            }
            infer_pattern_var_value(var, ast)?.parse::<i128>().ok()
        }
        Expr::Negate(inner) => Some(-eval_expr_to_literal_with_locals(inner, ast, locals)?),
        Expr::Binary(bin) if !bin.op.is_comparison() => {
            let lhs = eval_expr_to_literal_with_locals(&bin.lhs, ast, locals)?;
            let rhs = eval_expr_to_literal_with_locals(&bin.rhs, ast, locals)?;
            match bin.op {
                BinOp::Add => Some(lhs + rhs),
                BinOp::Subtract => Some(lhs - rhs),
                BinOp::Multiply => Some(lhs * rhs),
                BinOp::Divide if rhs != 0 => Some(lhs / rhs),
                BinOp::Modulo if rhs != 0 => Some(lhs % rhs),
                _ => None,
            }
        }
        _ => None,
    }
}

fn compute_rich_return_type(bind: &Bind, ast: &FileAst) -> Option<String> {
    let BindValue::Body { exprs, ret } = bind.value() else {
        return None;
    };
    let mut types: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let push =
        |types: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, val: String| {
            if seen.insert(val.clone()) {
                types.push(val);
            }
        };
    match &ret.0 {
        None => push(&mut types, &mut seen, "Nothing".to_string()),
        Some(expr) => {
            let outer_locals: Vec<&Expr> = exprs.iter().map(|e| &**e).collect();
            if let Some(v) = eval_expr_to_literal_with_locals(expr, ast, &outer_locals) {
                push(&mut types, &mut seen, v.to_string());
            }
        }
    }
    for spanned_expr in exprs {
        if let Expr::If(if_expr) = &**spanned_expr
            && let Some(ret_expr) = if_expr.ret.0.as_ref()
        {
            let combined: Vec<&Expr> = exprs
                .iter()
                .map(|e| &**e)
                .chain(if_expr.body.iter().map(|e| &**e))
                .collect();
            if let Some(v) = eval_expr_to_literal_with_locals(ret_expr, ast, &combined) {
                push(&mut types, &mut seen, v.to_string());
            }
        }
    }
    if types.is_empty() {
        return None;
    }
    Some(types.join(" or "))
}

fn find_local_bind_recursive<'a>(exprs: &'a [Spanned<Expr>], name: &str) -> Option<&'a Bind> {
    for expr in exprs {
        match &**expr {
            Expr::Bind(b) if b.name().as_str() == name => return Some(b),
            Expr::If(if_expr) => {
                if let Some(found) = find_local_bind_recursive(&if_expr.body, name) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn infer_pattern_var_value(var_name: &str, ast: &FileAst) -> Option<String> {
    for bind in ast.defs().values() {
        let BindValue::Body { exprs, .. } = bind.value() else {
            continue;
        };
        let pattern_info = exprs.iter().find_map(|e| {
            let Expr::If(if_expr) = &**e else { return None };
            let IfCondition::Pattern { subject, tag } = &if_expr.condition else {
                return None;
            };
            let params = match tag {
                Tag::Generic(_, params, _) => params,
                _ => return None,
            };
            let param_pos = params.keys().position(|k| k.as_str() == var_name)?;
            let subject_name = match &subject.as_ref().0 {
                Expr::FnCall(c) if c.args.is_none() && c.path.segments.is_empty() => {
                    c.path.root.as_str()
                }
                _ => return None,
            };
            Some((subject_name.to_string(), tag.name().to_string(), param_pos))
        });
        let (subject_name, variant_name, param_pos) = pattern_info?;
        let type_annotation = exprs.iter().find_map(|e| {
            let Expr::Bind(b) = &**e else { return None };
            if b.name().as_str() == subject_name {
                b.type_annotation.as_ref()
            } else {
                None
            }
        })?;
        let (type_name, type_args) = type_annotation;
        let union_decl = ast
            .tags()
            .iter()
            .find(|(k, _)| k.as_str() == type_name.as_str())?
            .1;
        let DeclareValue::Union { variants } = union_decl.value() else {
            return None;
        };
        for variant in variants {
            let vtag = variant.tag();
            if vtag.name() != variant_name {
                continue;
            }
            let Tag::Generic(_, variant_params, _) = vtag else {
                continue;
            };
            let union_param_name = variant_params.keys().nth(param_pos)?;
            let type_param_pos = union_decl
                .params()
                .as_ref()?
                .keys()
                .position(|k| k == union_param_name)?;
            let arg = type_args.get(type_param_pos)?;
            return Some(match &**arg {
                Expr::Lit(Literal::Int(n)) => n.to_string(),
                Expr::Lit(Literal::Number(n)) => n.to_string(),
                Expr::Lit(Literal::Float(f)) => f.to_string(),
                _ => return None,
            });
        }
    }
    None
}

fn enclosing_method_receiver(source: &str, cursor_byte: usize) -> Option<&str> {
    let before = &source[..cursor_byte];
    for line in before.lines().rev() {
        let trimmed = line.trim_start();
        if let Some(dot_pos) = trimmed.find('.') {
            let type_part = &trimmed[..dot_pos];
            if type_part
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
                && type_part.chars().all(|c| c.is_alphanumeric())
            {
                return Some(type_part);
            }
        }
    }
    None
}

fn is_in_tag_params(source: &str, cursor_byte: usize) -> bool {
    let before = &source[..cursor_byte.min(source.len())];
    let bytes = before.as_bytes();
    let mut depth = 0i32;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    let before_paren = before[..i].trim_end();
                    let id_end = before_paren.len();
                    let id_start = before_paren
                        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    let ident = &before_paren[id_start..id_end];
                    return ident
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                } else {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    false
}

fn format_params(params: &crate::ast::Parameters) -> String {
    if params.is_empty() {
        return String::new();
    }
    let mut s = String::from("(");
    let mut first = true;
    for (name, kind) in params {
        if !first {
            s.push_str(", ");
        }
        first = false;
        s.push_str(name.as_str());
        match kind {
            ParameterKind::Tagged(tag) => {
                s.push(' ');
                s.push_str(&tag.to_string());
            }
            ParameterKind::Generic | ParameterKind::Default(_) => {}
        }
    }
    s.push(')');
    s
}
