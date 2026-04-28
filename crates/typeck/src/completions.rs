//! Completions and signature-help (semantic layer, no LSP types).

use ast::{
    BindValue, Expr, FileAst, FormatPart, HasSpanId, IfCondition, LoopEnum, ParameterKind,
    Parameters, SpanTable, WhenArm,
};

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
}

pub fn completions_for_ast(ast: &FileAst) -> Vec<CompletionCandidate> {
    let mut items = Vec::new();

    for (name, decl) in ast.tags() {
        let detail = decl
            .params()
            .as_ref()
            .map(|p| format!("tag {}{}", name, format_params(p)));
        let documentation = decl.doc_comment().map(|dc| dc.0.clone());
        items.push(CompletionCandidate {
            label: name.to_string(),
            kind: CompletionKind::Tag,
            detail,
            documentation,
        });
    }

    for (name, bind) in ast.defs() {
        let is_fn = bind.params().is_some();
        let mut detail = bind
            .params()
            .as_ref()
            .map(|p| format!("{}{}", name.as_str(), format_params(p)));
        if let Some(complexity) = bind.attributes().complexity.as_ref() {
            let complexity_str = format!("complexity = {}", complexity.display_big_o());
            detail = Some(match detail {
                Some(d) => format!("{}\n{}", d, complexity_str),
                None => complexity_str,
            });
        }
        let documentation = bind.doc_comment().map(|dc| dc.0.clone());
        items.push(CompletionCandidate {
            label: name.as_str().to_string(),
            kind: if is_fn {
                CompletionKind::Function
            } else {
                CompletionKind::Variable
            },
            detail,
            documentation,
        });
    }

    // TODO: fix keywords here, make a single source of truth
    for kw in ["if", "else", "for", "in", "while", "return", "use", "tag"] {
        items.push(CompletionCandidate {
            label: kw.to_string(),
            kind: CompletionKind::Keyword,
            detail: None,
            documentation: None,
        });
    }

    items
}

#[derive(Debug, Clone)]
pub struct SignatureInfo {
    pub label: String,
    pub params: Vec<String>,
    pub documentation: Option<String>,
}

pub fn signature_for_fn(ast: &FileAst, fn_name: &str) -> Option<SignatureInfo> {
    let (_, bind) = ast
        .defs()
        .iter()
        .find(|(name, _)| name.as_str() == fn_name)?;
    let params = bind.params().as_ref()?;
    Some(SignatureInfo {
        label: format!("{}{}", fn_name, format_params(params)),
        params: params.keys().map(|p| p.to_string()).collect(),
        documentation: bind.doc_comment().map(|dc| dc.0.clone()),
    })
}

/// Find the name of the innermost function call whose argument list contains `byte_pos`.
///
/// Used by LSP signature help: given a cursor position, returns the function name
/// so the caller can look up its signature.
pub fn fn_call_at(ast: &FileAst, byte_pos: usize) -> Option<String> {
    let span_table = ast.span_table();
    let mut best: Option<(String, usize)> = None;
    for (expr, span_id) in ast.top_level_exprs() {
        find_call_in_expr(expr, *span_id, span_table, byte_pos, &mut best);
    }
    for bind in ast.defs().values() {
        find_call_in_bind_value(bind.value(), span_table, byte_pos, &mut best);
    }
    best.map(|(name, _)| name)
}

fn find_call_in_type_surface(
    expr: &Expr,
    span_id: ast::SpanId,
    span_table: &SpanTable,
    byte_pos: usize,
    best: &mut Option<(String, usize)>,
) {
    let span = span_table.get(span_id);
    if byte_pos < span.start || byte_pos > span.end {
        return;
    }
    match expr {
        Expr::TypeGeneric { params, .. } => {
            for (_, pk) in params {
                match pk {
                    ParameterKind::Default(e) => {
                        find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
                    }
                    ParameterKind::Tagged(sp) => {
                        find_call_in_type_surface(&sp.0, sp.1, span_table, byte_pos, best);
                    }
                    ParameterKind::Generic => {}
                }
            }
        }
        _ => {}
    }
}

fn find_call_in_expr(
    expr: &Expr,
    span_id: ast::SpanId,
    span_table: &SpanTable,
    byte_pos: usize,
    best: &mut Option<(String, usize)>,
) {
    let span = span_table.get(span_id);
    if byte_pos < span.start || byte_pos > span.end {
        return;
    }
    match expr {
        Expr::FnCall(call) if call.args.is_some() => {
            let call_span = span_table.get(call.path.span_id());
            if call_span.end <= byte_pos {
                let len = span.end - span.start;
                if best.as_ref().is_none_or(|(_, bl)| len < *bl) {
                    *best = Some((call.path.root.as_str().to_string(), len));
                }
            }
            if let Some(args) = &call.args {
                for arg in args {
                    find_call_in_expr(&arg.0, arg.1, span_table, byte_pos, best);
                }
            }
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    find_call_in_expr(&arg.0, arg.1, span_table, byte_pos, best);
                }
            }
        }
        Expr::Binary(bin) => {
            find_call_in_expr(&bin.lhs.0, bin.lhs.1, span_table, byte_pos, best);
            find_call_in_expr(&bin.rhs.0, bin.rhs.1, span_table, byte_pos, best);
        }
        Expr::Bind(bind) => find_call_in_bind_value(bind.value(), span_table, byte_pos, best),
        Expr::When(w) => {
            if let Some(s) = &w.subject {
                find_call_in_expr(&s.0, s.1, span_table, byte_pos, best);
            }
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond { condition, body } => {
                        find_call_in_expr(&condition.0, condition.1, span_table, byte_pos, best);
                        find_call_in_expr(&body.0, body.1, span_table, byte_pos, best);
                    }
                    WhenArm::Is { pattern, body } => {
                        find_call_in_expr(&pattern.0, pattern.1, span_table, byte_pos, best);
                        find_call_in_expr(&body.0, body.1, span_table, byte_pos, best);
                    }
                    WhenArm::Else(body) => {
                        find_call_in_expr(&body.0, body.1, span_table, byte_pos, best);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            match &if_expr.condition {
                IfCondition::Bool(e) => find_call_in_expr(&e.0, e.1, span_table, byte_pos, best),
                IfCondition::Pattern { subject, pattern } => {
                    find_call_in_expr(&subject.0, subject.1, span_table, byte_pos, best);
                    find_call_in_expr(&pattern.0, pattern.1, span_table, byte_pos, best);
                }
            }
            for e in &if_expr.body {
                find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            LoopEnum::ForIn(fl) => {
                find_call_in_expr(&fl.iter.0, fl.iter.1, span_table, byte_pos, best);
                for e in &fl.exprs {
                    find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
                }
            }
            LoopEnum::While(wl) => {
                find_call_in_expr(&wl.cond.0, wl.cond.1, span_table, byte_pos, best);
                for e in &wl.exprs {
                    find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let FormatPart::Expr(e) = part {
                    find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
                }
            }
        }
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                find_call_in_expr(&arg.0, arg.1, span_table, byte_pos, best);
            }
        }
        Expr::Range(r) => {
            find_call_in_expr(&r.start.0, r.start.1, span_table, byte_pos, best);
            find_call_in_expr(&r.end.0, r.end.1, span_table, byte_pos, best);
        }
        Expr::TupleAlloc { init, .. } => {
            find_call_in_expr(&init.0, init.1, span_table, byte_pos, best)
        }
        Expr::TupleGet { base, .. } => {
            find_call_in_expr(&base.0, base.1, span_table, byte_pos, best)
        }
        Expr::TupleSet { base, value, .. } => {
            find_call_in_expr(&base.0, base.1, span_table, byte_pos, best);
            find_call_in_expr(&value.0, value.1, span_table, byte_pos, best);
        }
        Expr::Cast { expr, .. } => find_call_in_expr(&expr.0, expr.1, span_table, byte_pos, best),
        Expr::BufGet { buf, index, .. } => {
            find_call_in_expr(&buf.0, buf.1, span_table, byte_pos, best);
            find_call_in_expr(&index.0, index.1, span_table, byte_pos, best);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            find_call_in_expr(&buf.0, buf.1, span_table, byte_pos, best);
            find_call_in_expr(&index.0, index.1, span_table, byte_pos, best);
            find_call_in_expr(&value.0, value.1, span_table, byte_pos, best);
        }
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
        }
        Expr::TupleLit(elems) => {
            for e in elems {
                find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
            }
        }
        Expr::TypeGeneric { span, .. } => {
            find_call_in_type_surface(expr, *span, span_table, byte_pos, best);
        }
        Expr::TypeNominal(..) | Expr::TypeQualified(_) => {}
        Expr::Lit(_) | Expr::SelfRef(_) | Expr::AnonymousTag(..) | Expr::Asm(_) => {}
    }
}

fn find_call_in_bind_value(
    value: &BindValue,
    span_table: &SpanTable,
    byte_pos: usize,
    best: &mut Option<(String, usize)>,
) {
    match value {
        BindValue::Expr(e) => find_call_in_expr(&e.0, e.1, span_table, byte_pos, best),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                find_call_in_expr(&e.0, e.1, span_table, byte_pos, best);
            }
            if let Some(r) = &ret.0 {
                find_call_in_expr(&r.0, r.1, span_table, byte_pos, best);
            }
        }
        BindValue::Extern => {}
    }
}

pub fn format_params(params: &Parameters) -> String {
    if params.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = params
        .iter()
        .map(|(name, kind)| match kind {
            ParameterKind::Generic => name.to_string(),
            ParameterKind::Tagged(_) => format!("{name}{kind}"),
            ParameterKind::Default(expr) => format!("{name}: {expr:?}"),
        })
        .collect();
    format!("({})", parts.join(", "))
}
