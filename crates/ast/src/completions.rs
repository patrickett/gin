//! Completions and signature-help (semantic layer, no LSP types).

use crate::{
    BindValue, Expr, FileAst, FormatPart, IfCondition, LoopEnum, ParameterKind, Parameters,
    SpanTable, TypeExpr, WhenArm,
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
        let documentation = decl.doc_comment().map(|dc| dc.value.clone());
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
        let documentation = bind.doc_comment().map(|dc| dc.value.clone());
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
        documentation: bind.doc_comment().map(|dc| dc.value.clone()),
    })
}

/// Find the name of the innermost function call whose argument list contains `byte_pos`.
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
    expr: &TypeExpr,
    span_id: crate::SpanId,
    span_table: &SpanTable,
    byte_pos: usize,
    best: &mut Option<(String, usize)>,
) {
    let span = span_table.get(span_id);
    if byte_pos < span.start || byte_pos > span.end {
        return;
    }
    if let TypeExpr::Generic { params, .. } = expr {
        for (_, pk) in params {
            match pk {
                ParameterKind::Default(e) => {
                    find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
                }
                ParameterKind::Tagged(sp) => {
                    if let Some(te) = sp.value.as_type_expr() {
                        find_call_in_type_surface(&te, sp.span_id, span_table, byte_pos, best);
                    }
                }
                ParameterKind::Generic => {}
            }
        }
    }
}

fn find_call_in_expr(
    expr: &Expr,
    span_id: crate::SpanId,
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
                    find_call_in_expr(&arg.value, arg.span_id, span_table, byte_pos, best);
                }
            }
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    find_call_in_expr(&arg.value, arg.span_id, span_table, byte_pos, best);
                }
            }
        }
        Expr::Binary(bin) => {
            find_call_in_expr(&bin.lhs.value, bin.lhs.span_id, span_table, byte_pos, best);
            find_call_in_expr(&bin.rhs.value, bin.rhs.span_id, span_table, byte_pos, best);
        }
        Expr::Bind(bind) => find_call_in_bind_value(bind.value(), span_table, byte_pos, best),
        Expr::When(w) => {
            if let Some(s) = &w.subject {
                find_call_in_expr(&s.value, s.span_id, span_table, byte_pos, best);
            }
            for arm in &w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        find_call_in_expr(
                            &condition.value,
                            condition.span_id,
                            span_table,
                            byte_pos,
                            best,
                        );
                        find_call_in_expr(&body.value, body.span_id, span_table, byte_pos, best);
                    }
                    WhenArm::Is { pattern, body, .. } => {
                        find_call_in_type_surface(
                            &pattern.value,
                            pattern.span_id,
                            span_table,
                            byte_pos,
                            best,
                        );
                        find_call_in_expr(&body.value, body.span_id, span_table, byte_pos, best);
                    }
                    WhenArm::Else(body, _) => {
                        find_call_in_expr(&body.value, body.span_id, span_table, byte_pos, best);
                    }
                }
            }
        }
        Expr::If(if_expr) => {
            match &if_expr.condition {
                IfCondition::Bool(e) => {
                    find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best)
                }
                IfCondition::Pattern { subject, .. } => {
                    find_call_in_expr(&subject.value, subject.span_id, span_table, byte_pos, best);
                }
            }
            for e in &if_expr.body {
                find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
            }
        }
        Expr::Loop(loop_expr) => match loop_expr {
            LoopEnum::ForIn(fl) => {
                find_call_in_expr(&fl.iter.value, fl.iter.span_id, span_table, byte_pos, best);
                for e in &fl.exprs {
                    find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
                }
            }
            LoopEnum::While(wl) => {
                find_call_in_expr(&wl.cond.value, wl.cond.span_id, span_table, byte_pos, best);
                for e in &wl.exprs {
                    find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
                }
            }
        },
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let FormatPart::Expr(e, _) = part {
                    find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
                }
            }
        }
        Expr::TagCall(tc) => {
            for arg in &tc.args {
                find_call_in_expr(&arg.value, arg.span_id, span_table, byte_pos, best);
            }
        }
        Expr::Range(r) => {
            find_call_in_expr(&r.start.value, r.start.span_id, span_table, byte_pos, best);
            find_call_in_expr(&r.end.value, r.end.span_id, span_table, byte_pos, best);
        }
        Expr::TupleAlloc { init, .. } => {
            find_call_in_expr(&init.value, init.span_id, span_table, byte_pos, best)
        }
        Expr::TupleGet { base, .. } => {
            find_call_in_expr(&base.value, base.span_id, span_table, byte_pos, best)
        }
        Expr::TupleSet { base, value, .. } => {
            find_call_in_expr(&base.value, base.span_id, span_table, byte_pos, best);
            find_call_in_expr(&value.value, value.span_id, span_table, byte_pos, best);
        }
        Expr::Cast { expr, .. } => {
            find_call_in_expr(&expr.value, expr.span_id, span_table, byte_pos, best)
        }
        Expr::BufGet { buf, index, .. } => {
            find_call_in_expr(&buf.value, buf.span_id, span_table, byte_pos, best);
            find_call_in_expr(&index.value, index.span_id, span_table, byte_pos, best);
        }
        Expr::BufSet {
            buf, index, value, ..
        } => {
            find_call_in_expr(&buf.value, buf.span_id, span_table, byte_pos, best);
            find_call_in_expr(&index.value, index.span_id, span_table, byte_pos, best);
            find_call_in_expr(&value.value, value.span_id, span_table, byte_pos, best);
        }
        Expr::TakePtr(e)
        | Expr::TakeRef(e)
        | Expr::Deref(e)
        | Expr::Negate(e)
        | Expr::MutArg(e)
        | Expr::OwnArg(e) => {
            find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
            }
        }
        Expr::Lit(_)
        | Expr::SelfRef
        | Expr::AnonymousTag(..)
        | Expr::Asm(_)
        | Expr::TypeNominal(..)
        | Expr::TypeQualified(_)
        | Expr::TypeGeneric { .. } => {}
    }
}

fn find_call_in_bind_value(
    value: &BindValue,
    span_table: &SpanTable,
    byte_pos: usize,
    best: &mut Option<(String, usize)>,
) {
    match value {
        BindValue::Expr(e) => find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                find_call_in_expr(&e.value, e.span_id, span_table, byte_pos, best);
            }
            if let Some(r) = &ret.value {
                find_call_in_expr(&r.value, r.span_id, span_table, byte_pos, best);
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
        .map(|(name, kind)| {
            if let ParameterKind::Tagged(sp) = kind
                && let Some(te) = sp.value.as_type_expr()
                && let TypeExpr::Nominal(type_name, _) = &te
                && name.eq_ignore_ascii_case(type_name.as_str())
            {
                let short = name.chars().next().unwrap_or('p').to_string();
                return format!("{short}{kind}");
            }
            match kind {
                ParameterKind::Generic => name.to_string(),
                ParameterKind::Tagged(_) => format!("{name}{kind}"),
                ParameterKind::Default(expr) => format!("{name}: {expr:?}"),
            }
        })
        .collect();
    format!("({})", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::SpanId;
    use crate::{Literal, Spanned, Typed};
    use indexmap::IndexMap;
    use internment::Intern;

    fn intern(s: &str) -> Intern<String> {
        Intern::new(s.to_owned())
    }

    fn make_params(items: Vec<(Intern<String>, ParameterKind)>) -> Parameters {
        let mut map = IndexMap::new();
        for (name, kind) in items {
            map.insert(name, kind);
        }
        map
    }

    fn tagged_param(name: &str, type_name: &str) -> (Intern<String>, ParameterKind) {
        let expr = Expr::TypeNominal(intern(type_name));
        (
            intern(name),
            ParameterKind::Tagged(Box::new(Spanned {
                value: expr,
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
        assert_eq!(
            format_params(&params),
            "(x: Typed { value: Lit(Number(42)), ty: Infer, const_value: None, span_id: SpanId(0), flaws: [] })"
        );
    }
}
