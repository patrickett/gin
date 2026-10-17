use diagnostic::code::FormatCode;
use std::ops::ControlFlow;

use ast::folder::{Visitor, walk_bind};
use ast::{Bind, BindValue, Expr};
use diagnostic::{Category, Diagnostic, Span};

use super::{Edit, Help, HelpContext};

impl HelpContext<'_> {
    pub(super) fn direct_bind_suggestions(&self) -> Vec<Help> {
        suggestions(self.source, self.parsed)
    }
}

pub(crate) fn format_value_bodies(source: &str, parsed: &parser::query::ParseOutput) -> String {
    let mut formatted = source.to_owned();
    for edit in super::edits(&suggestions(source, parsed)) {
        formatted.replace_range(edit.span.to_range(), &edit.replacement);
    }
    formatted
}

fn suggestions(source: &str, parsed: &parser::query::ParseOutput) -> Vec<Help> {
    let mut visitor = DirectBindings {
        source,
        parsed,
        helps: Vec::new(),
    };
    let _ = visitor.visit_file_ast(&parsed.ast);
    visitor.helps
}

struct DirectBindings<'a> {
    source: &'a str,
    parsed: &'a parser::query::ParseOutput,
    helps: Vec<Help>,
}

impl Visitor for DirectBindings<'_> {
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        if bind.params.is_none()
            && let BindValue::Expr(body) = &bind.value
            && let Expr::Block(items) = &body.value
            && let [expression] = items.as_slice()
            && !matches!(
                expression.value,
                Expr::Return(_) | Expr::Bind(_) | Expr::Block(_)
            )
        {
            let spans = &self.parsed.ast.span_table;
            let header_start = spans.get(bind.name_span).start();
            let span = spans.get(expression.span_id);
            let header = &self.source[header_start..span.start()];
            if let Some(colon) = header.rfind(':')
                && header[colon + 1..].chars().all(char::is_whitespace)
                && !self.parsed.comments.iter().any(|comment| {
                    let comment = spans.get(comment.span);
                    comment.start() < span.end() && comment.end() > header_start
                })
            {
                let start = header_start + colon + 1;
                self.helps.push(Help {
                    // fmt-single-expression-body
                    diagnostic: Diagnostic::new(
                        FormatCode::SingleExpressionBody,
                        "value block contains a single expression",
                    )
                    .with_category(Category::Help)
                    .with_help("Bind expression directly")
                    .at_span(Span::from_range(start..span.end())),
                    edits: vec![Edit {
                        span: Span::from_range(start..span.end()),
                        replacement: format!(" {}", &self.source[span.to_range()]),
                    }],
                });
            }
        }
        walk_bind(self, bind)
    }
}
