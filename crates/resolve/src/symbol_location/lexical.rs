use std::ops::{ControlFlow, Range};

use ast::span::{SpanId, SpanTable};
use ast::{
    BindValue, Expr, FileAst, HasMember, HasMemberBody, Loop, ParameterKind, Parameters, Typed,
};
use internment::Intern;

pub fn definition_span(ast: &FileAst, word: &str, byte: usize) -> Option<Range<usize>> {
    let lookup = Lookup {
        spans: &ast.span_table,
        name: Intern::from_ref(word),
        byte,
    };
    for bind in ast.defs.values() {
        if let Some(span) = lookup.in_function(bind.params.as_ref(), Some(&bind.value)) {
            return Some(lookup.range(span));
        }
    }
    for decl in ast.tags.values() {
        let ast::DeclareValue::Has(members) = &decl.value else {
            continue;
        };
        for member in members {
            let HasMember::Function(function) = member else {
                continue;
            };
            let body = function.body.as_ref().map(|body| match body {
                HasMemberBody::Overrideable(value) | HasMemberBody::Final(value) => value,
            });
            if let Some(span) = lookup.in_function(Some(&function.params), body) {
                return Some(lookup.range(span));
            }
            if word == "Self"
                && (body.is_some_and(|body| lookup.body_contains(body))
                    || function
                        .return_ty
                        .as_ref()
                        .is_some_and(|ty| lookup.spans.contains(ty.span_id, byte))
                    || function.params.values().any(|param| match &param.kind {
                        ParameterKind::Tagged(ty)
                        | ParameterKind::ValueParam { ty }
                        | ParameterKind::Inferred { ty } => lookup.spans.contains(ty.span_id, byte),
                        _ => false,
                    }))
            {
                return Some(lookup.range(decl.name_span));
            }
        }
    }
    None
}

struct Lookup<'a> {
    spans: &'a SpanTable,
    name: Intern<String>,
    byte: usize,
}

impl Lookup<'_> {
    fn range(&self, span: SpanId) -> Range<usize> {
        let span = self.spans.get(span);
        span.start()..span.end()
    }

    fn contains(&self, expr: &Typed<Expr>) -> bool {
        self.spans.contains(expr.span_id, self.byte)
    }

    fn body_contains(&self, value: &BindValue) -> bool {
        match value {
            BindValue::Expr(expr) => self.contains(expr),
            BindValue::Body { exprs, ret } => exprs
                .iter()
                .chain(&ret.values)
                .any(|expr| self.contains(expr)),
            BindValue::Unassigned => false,
        }
    }

    fn in_function(
        &self,
        params: Option<&Parameters>,
        value: Option<&BindValue>,
    ) -> Option<SpanId> {
        if let Some(value) = value
            && self.body_contains(value)
            && let Some(span) = self.in_body(value)
        {
            return Some(span);
        }
        let in_body = value.is_some_and(|value| self.body_contains(value));
        for (name, param) in params.into_iter().flatten() {
            if *name == self.name && (in_body || self.spans.contains(param.name_span, self.byte)) {
                return Some(param.name_span);
            }
            if let Some(pattern) = &param.range_pattern
                && pattern.end == self.name
                && (in_body || self.spans.contains(pattern.end_span, self.byte))
            {
                return Some(pattern.end_span);
            }
            let ty = match &param.kind {
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => ty,
                _ => continue,
            };
            if let Some(span) = self.dependent_name(&ty.value)
                && (in_body || self.spans.contains(span, self.byte))
            {
                return Some(span);
            }
        }
        None
    }

    fn dependent_name(&self, expr: &Expr) -> Option<SpanId> {
        if let Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { count, .. }) = expr
            && let Expr::FnCall(call) = &count.value
            && call.path.is_bare()
            && call.args.is_none()
            && call.path.root == self.name
        {
            return Some(call.path.root_span);
        }
        let mut found = None;
        let _ = ast::folder::walk_expr_children(expr, &mut |_, child| {
            found = self.dependent_name(child);
            if found.is_some() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        found
    }

    fn in_body(&self, value: &BindValue) -> Option<SpanId> {
        match value {
            BindValue::Expr(expr) => self.in_expr(expr),
            BindValue::Body { exprs, ret } => self.in_sequence(exprs.iter().chain(&ret.values)),
            BindValue::Unassigned => None,
        }
    }

    fn in_sequence<'a>(&self, exprs: impl IntoIterator<Item = &'a Typed<Expr>>) -> Option<SpanId> {
        let mut visible = None;
        for expr in exprs {
            if self.contains(expr) {
                return self.in_expr(expr).or(visible);
            }
            if self.spans.get(expr.span_id).end() <= self.byte
                && let Expr::Bind(bind) = &expr.value
                && bind.name == self.name
                && matches!(
                    bind.operator,
                    ast::BindOperator::FreshMutable | ast::BindOperator::FreshImmutable
                )
            {
                visible = Some(bind.name_span);
            }
        }
        None
    }

    fn in_expr(&self, expr: &Typed<Expr>) -> Option<SpanId> {
        match &expr.value {
            Expr::If(branch) => {
                if branch
                    .body
                    .iter()
                    .chain(&branch.ret.values)
                    .any(|expr| self.contains(expr))
                {
                    return self.in_sequence(branch.body.iter().chain(&branch.ret.values));
                }
            }
            Expr::Loop(Loop::ForIn(loop_)) => {
                if loop_.exprs.iter().any(|expr| self.contains(expr)) {
                    return self.in_sequence(&loop_.exprs).or_else(|| {
                        (loop_.pat.value.for_loop_single_binding() == Some(self.name))
                            .then_some(loop_.pat.span_id)
                    });
                }
            }
            Expr::Loop(Loop::While(loop_)) => {
                if loop_.exprs.iter().any(|expr| self.contains(expr)) {
                    return self.in_sequence(&loop_.exprs);
                }
            }
            Expr::Bind(bind) => {
                if bind.name == self.name
                    && self.spans.contains(bind.name_span, self.byte)
                    && matches!(
                        bind.operator,
                        ast::BindOperator::FreshMutable | ast::BindOperator::FreshImmutable
                    )
                {
                    return Some(bind.name_span);
                }
                return self.in_function(bind.params.as_ref(), Some(&bind.value));
            }
            _ => {}
        }
        let mut found = None;
        let _ = ast::folder::walk_typed_expr_children(&expr.value, &mut |child| {
            if self.contains(child) {
                found = self.in_expr(child);
            }
            if found.is_some() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        });
        found
    }
}
