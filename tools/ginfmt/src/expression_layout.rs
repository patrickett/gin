use std::ops::ControlFlow;

use ast::folder::walk_expr_children;
use ast::{
    BindValue, CallSyntax, Declare, DeclareValue, Expr, FileAst, HasMember, HasMemberBody, SpanId,
    SpanTable, Typed, WhenArm,
};
use lexer::Token;

use crate::Config;

pub(crate) fn format(source: &str, ast: &FileAst, config: &Config) -> String {
    let mut roots = Vec::new();
    for bind in ast
        .defs
        .values()
        .filter(|bind| !ast.method_binds.contains(bind))
    {
        collect_value(&bind.value, &mut roots);
    }
    for declare in ast.tags.values() {
        collect_declare(declare, &mut roots);
    }
    roots.extend(ast.exprs.iter().map(|(expr, span)| (*span, expr)));
    Layout {
        source,
        spans: &ast.span_table,
        config,
    }
    .range(0, source.len(), roots, 0, 0)
}

fn collect_value<'a>(value: &'a BindValue, roots: &mut Vec<(SpanId, &'a Expr)>) {
    match value {
        BindValue::Expr(expr) => roots.push((expr.span_id, &expr.value)),
        BindValue::Body { exprs, ret } => {
            roots.extend(
                exprs
                    .iter()
                    .chain(&ret.values)
                    .map(|expr| (expr.span_id, &expr.value)),
            );
        }
        BindValue::Unassigned => {}
    }
}

fn collect_declare<'a>(declare: &'a Declare, roots: &mut Vec<(SpanId, &'a Expr)>) {
    if let DeclareValue::Has(members) = &declare.value {
        for member in members {
            let body = match member {
                HasMember::Property(property) => &property.body,
                HasMember::Function(function) => &function.body,
            };
            if let Some(HasMemberBody::Overrideable(value) | HasMemberBody::Final(value)) = body {
                collect_value(value, roots);
            }
        }
    }
    for extension in &declare.extensions {
        collect_declare(extension, roots);
    }
}

struct Layout<'a> {
    source: &'a str,
    spans: &'a SpanTable,
    config: &'a Config,
}

/// A source range together with the layout coordinates it is rendered at.
struct LayoutSpan {
    start: usize,
    end: usize,
    column: usize,
    indent: usize,
}

impl Layout<'_> {
    fn range(
        &self,
        start: usize,
        end: usize,
        mut children: Vec<(SpanId, &Expr)>,
        column: usize,
        indent: usize,
    ) -> String {
        children.sort_by_key(|(span, _)| self.spans.get(*span).start());
        let mut text = String::new();
        let mut position = start;
        for (span, expr) in children {
            let span = self.spans.get(span);
            if span.is_empty() || span.start() < position || span.end() > end {
                continue;
            }
            text.push_str(&self.source[position..span.start()]);
            let (child_column, child_indent) = line_position(&text, column, indent);
            text.push_str(&self.expr(expr, span.start(), span.end(), child_column, child_indent));
            position = span.end();
        }
        text.push_str(&self.source[position..end]);
        text
    }

    fn children<'a>(&self, expr: &'a Expr) -> Vec<(SpanId, &'a Expr)> {
        let mut children = Vec::new();
        let _ = walk_expr_children(expr, &mut |span, child| {
            children.push((span, child));
            ControlFlow::Continue(())
        });
        children
    }

    fn expr(&self, expr: &Expr, start: usize, end: usize, column: usize, indent: usize) -> String {
        let args = match expr {
            Expr::TagCall(call) => Some(call.args.as_slice()),
            Expr::FnCall(call) => call.args.as_deref().map(|args| {
                if call.syntax == CallSyntax::Receiver {
                    args.get(1..).unwrap_or_default()
                } else {
                    args
                }
            }),
            _ => None,
        };
        if let Some(args) = args.filter(|args| !args.is_empty())
            && let Some(text) = self.call(expr, args, start, end, column, indent)
        {
            return text;
        }
        if let Expr::When(when) = expr
            && let [first, WhenArm::Else(otherwise, _)] = when.arms.as_slice()
        {
            let body = match first {
                WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } => Some(body.as_ref()),
                WhenArm::Else(..) => None,
            };
            if let Some(body) = body
                && let Some(text) = self.when(
                    expr,
                    body,
                    otherwise,
                    LayoutSpan {
                        start,
                        end,
                        column,
                        indent,
                    },
                )
            {
                return text;
            }
        }
        self.range(start, end, self.children(expr), column, indent)
    }

    fn typed(&self, expr: &Typed<Expr>, column: usize, indent: usize) -> String {
        let span = self.spans.get(expr.span_id);
        self.expr(&expr.value, span.start(), span.end(), column, indent)
    }

    fn call(
        &self,
        expr: &Expr,
        args: &[Typed<Expr>],
        start: usize,
        end: usize,
        column: usize,
        indent: usize,
    ) -> Option<String> {
        let first = self.spans.get(args.first()?.span_id);
        let last = self.spans.get(args.last()?.span_id);
        if first.start() < start || last.end() > end {
            return None;
        }
        let open = start + self.source[start..first.start()].rfind('(')?;
        let close = last.end() + self.source[last.end()..end].find(')')?;
        let head = self.range(start, open + 1, self.children(expr), column, indent);
        let tail = &self.source[close..end];
        let mut inline = head.clone();
        for (index, arg) in args.iter().enumerate() {
            if index > 0 {
                inline.push_str(", ");
            }
            inline.push_str(&self.typed(arg, column + inline.chars().count(), indent));
        }
        inline.push_str(tail);
        if !self.source[open..close].contains('\n')
            && !inline.contains('\n')
            && column + inline.chars().count() <= self.config.max_line_width
        {
            return Some(inline);
        }
        let mut text = head;
        for arg in args {
            text.push('\n');
            text.extend(std::iter::repeat_n(' ', indent + 4));
            text.push_str(&self.typed(arg, indent + 4, indent + 4));
        }
        text.push('\n');
        text.extend(std::iter::repeat_n(' ', indent));
        text.push_str(tail);
        Some(text)
    }

    fn when(
        &self,
        expr: &Expr,
        body: &Typed<Expr>,
        otherwise: &Typed<Expr>,
        span: LayoutSpan,
    ) -> Option<String> {
        let LayoutSpan {
            start,
            end,
            column,
            indent,
        } = span;
        if matches!(body.value, Expr::Block(_)) || matches!(otherwise.value, Expr::Block(_)) {
            return None;
        }
        let body_span = self.spans.get(body.span_id);
        let else_span = self.spans.get(otherwise.span_id);
        if body_span.start() < start || else_span.end() > end {
            return None;
        }
        let header = &self.source[start..body_span.start()];
        let mut tokens = lexer::Lexer::new(header);
        let mut then_start = None;
        let mut when_start = None;
        let mut indented = false;
        while let Some((token, span)) = tokens.next() {
            if indented && token != Token::Then {
                return None;
            }
            indented = token == Token::Indent;
            if token == Token::When && when_start.is_none() {
                when_start = Some(tokens.get_span(span).start());
            }
            if token == Token::Then {
                then_start = Some(start + tokens.get_span(span).start());
            }
        }
        let then_start = then_start?;
        let when_column = line_position(&header[..when_start?], column, indent).0;
        let prefix = self
            .range(start, then_start, self.children(expr), column, indent)
            .trim_end()
            .to_owned();
        let render = |continuation| {
            self.when_branches(&prefix, body, otherwise, else_span.end(), end, continuation)
        };
        let fits = |text: &str| {
            text.lines().enumerate().all(|(index, line)| {
                line.chars().count() + if index == 0 { column } else { 0 }
                    <= self.config.max_line_width
            })
        };
        let aligned = render(when_column);
        if fits(&aligned) {
            return Some(aligned);
        }
        let continuation = if when_column > indent + 4 {
            indent + 4
        } else {
            indent
        };
        let fallback = render(continuation);
        if fits(&fallback) || continuation == indent {
            Some(fallback)
        } else {
            Some(render(indent))
        }
    }

    fn when_branches(
        &self,
        prefix: &str,
        body: &Typed<Expr>,
        otherwise: &Typed<Expr>,
        suffix_start: usize,
        end: usize,
        continuation: usize,
    ) -> String {
        let mut text = prefix.to_owned();
        text.push('\n');
        text.extend(std::iter::repeat_n(' ', continuation));
        text.push_str("then ");
        text.push_str(&self.typed(body, continuation + 5, continuation));
        text.push('\n');
        text.extend(std::iter::repeat_n(' ', continuation));
        text.push_str("else ");
        text.push_str(&self.typed(otherwise, continuation + 5, continuation));
        text.push_str(&self.source[suffix_start..end]);
        text
    }
}

fn line_position(text: &str, column: usize, indent: usize) -> (usize, usize) {
    match text.rsplit_once('\n') {
        Some((_, line)) => (
            line.chars().count(),
            line.chars().take_while(|ch| *ch == ' ').count(),
        ),
        None => (column + text.chars().count(), indent),
    }
}
