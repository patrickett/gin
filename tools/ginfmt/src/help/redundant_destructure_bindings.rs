use diagnostic::code::FormatCode;
use std::collections::HashSet;
use std::ops::ControlFlow;

use ast::folder::{
    Visitor, walk_bind, walk_bind_value, walk_condition, walk_expr, walk_for_in, walk_if,
    walk_while,
};
use ast::{
    Bind, BindOperator, BindValue, Declare, DeclareValue, Expr, HasMember, HasMemberBody,
    ParamConvention, Typed,
};
use diagnostic::{Category, Diagnostic, Span};
use internment::Intern;
use lexer::{Lexer, Token};
use typecheck::TypedExprKind;

use super::{Edit, Help, HelpContext};

impl HelpContext<'_> {
    pub fn redundant_destructure_binding_suggestions(&self) -> Vec<Help> {
        if self
            .parsed
            .symptoms
            .iter()
            .any(|symptom| symptom.category == Category::Flaw)
        {
            return Vec::new();
        }
        let mut visitor = RedundantBindings {
            context: self,
            helps: Vec::new(),
            owned_receiver: false,
        };
        let _ = visitor.visit_file_ast(&self.parsed.ast);
        for declaration in self.parsed.ast.tags.values() {
            visitor.declaration(declaration);
        }
        visitor.helps
    }
}

struct RedundantBindings<'a, 'b> {
    context: &'a HelpContext<'b>,
    helps: Vec<Help>,
    owned_receiver: bool,
}

impl RedundantBindings<'_, '_> {
    fn declaration(&mut self, declaration: &Declare) {
        let mut pending = vec![(declaration, false)];
        while let Some((declaration, inherited_record)) = pending.pop() {
            let record = inherited_record
                || matches!(&declaration.value, DeclareValue::Has(members)
                if members.iter().any(|member| matches!(member,
                    HasMember::Property(property) if property.body.is_none())));
            if let DeclareValue::Has(members) = &declaration.value {
                for member in members {
                    let previous = self.owned_receiver;
                    self.owned_receiver = match member {
                        HasMember::Function(function) if record => function
                            .params
                            .get(&Intern::from_ref("self"))
                            .filter(|_| {
                                matches!(
                                    function.conventions.get(&Intern::from_ref("self")),
                                    None | Some(ParamConvention::Terminal | ParamConvention::Own)
                                )
                            })
                            .is_some(),
                        _ => false,
                    };
                    let body = match member {
                        HasMember::Function(function) => &function.body,
                        HasMember::Property(property) => &property.body,
                    };
                    if let Some(HasMemberBody::Overrideable(body) | HasMemberBody::Final(body)) =
                        body
                    {
                        if self.owned_receiver {
                            let mut shadowing = ReceiverShadow::default();
                            let _ = shadowing.visit_bind_value(body);
                            self.owned_receiver &= !shadowing.found;
                        }
                        let _ = self.visit_bind_value(body);
                    }
                    self.owned_receiver = previous;
                }
            }
            pending.extend(
                declaration
                    .extensions
                    .iter()
                    .map(|extension| (extension, record)),
            );
        }
    }

    fn sequence(&mut self, expressions: &[Typed<Expr>], end: usize) {
        for (index, expression) in expressions.iter().enumerate() {
            let Expr::Destructure { field_bindings, .. } = &expression.value else {
                continue;
            };
            let mut available = self.mutable_bindings(expression);
            for following in &expressions[index + 1..] {
                let Some((bind, source)) = plain_alias(&following.value) else {
                    break;
                };
                if !available.remove(&source) {
                    break;
                }
                let Some((field, source)) = field_bindings
                    .iter()
                    .find(|(_, binding)| *binding == source)
                else {
                    break;
                };
                self.helps.push(Help {
                    // fmt-redundant-destructure-binding
                    diagnostic: Diagnostic::new(
                        FormatCode::RedundantDestructureBinding,
                        format!(
                            "binding `{}` only renames destructured field `{source}`",
                            bind.name
                        ),
                    )
                    .with_category(Category::Help)
                    .with_arg("binding", bind.name.to_string())
                    .with_arg("source", source.to_string())
                    .with_help(format!(
                        "Bind `{field}: {}` in the destructuring pattern and remove the alias",
                        bind.name
                    ))
                    .at_span(self.context.parsed.ast.span_table.get(bind.name_span)),
                    edits: self.alias_edits(expression, bind, *field, *source, end),
                });
            }
        }
    }

    fn mutable_bindings(&self, expression: &Typed<Expr>) -> HashSet<Intern<String>> {
        let span = self.context.parsed.ast.span_table.get(expression.span_id);
        let typed = self.context.typed;
        let lowered = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                if typed.span_table.get(typed.exprs.span[index]) == span
                    && let TypedExprKind::Destructure { places, .. } = kind
                {
                    Some(places.iter().map(|place| &typed.places[place.0 as usize]))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        if lowered.is_empty() && self.owns_receiver(expression) {
            let Expr::Destructure { field_bindings, .. } = &expression.value else {
                unreachable!();
            };
            return field_bindings.iter().map(|(_, binding)| *binding).collect();
        }
        lowered
            .into_iter()
            .flatten()
            .filter(|place| place.mutable && place.parent.is_none())
            .map(|place| place.name)
            .collect()
    }

    fn owns_receiver(&self, expression: &Typed<Expr>) -> bool {
        if !self.owned_receiver {
            return false;
        }
        let Expr::Destructure { value, .. } = &expression.value else {
            return false;
        };
        matches!(value.value, Expr::SelfRef)
    }

    fn alias_edits(
        &self,
        destructure: &Typed<Expr>,
        bind: &Bind,
        field: Intern<String>,
        source: Intern<String>,
        end: usize,
    ) -> Vec<Edit> {
        let Expr::Destructure {
            field_bindings,
            value,
            ..
        } = &destructure.value
        else {
            unreachable!();
        };
        let spans = &self.context.parsed.ast.span_table;
        let BindValue::Expr(value_expression) = &bind.value else {
            unreachable!();
        };
        let alias_span = Span::new(
            spans.get(bind.name_span).start(),
            spans.get(value_expression.span_id).end(),
        );
        if field_bindings.iter().any(|(_, name)| *name == bind.name)
            || Lexer::new(&self.context.source[alias_span.end()..end])
                .any(|(token, _)| token == Token::Id(source.as_str()))
        {
            return Vec::new();
        }
        let pattern = Span::new(
            spans.get(destructure.span_id).start(),
            spans.get(value.span_id).start(),
        );
        let Some(rename) = pattern_edit(self.context.source, pattern, field, source, bind.name)
        else {
            return Vec::new();
        };
        let Some(removal) = alias_removal(self.context.source, alias_span) else {
            return Vec::new();
        };
        vec![rename, removal]
    }
}

impl Visitor for RedundantBindings<'_, '_> {
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        let previous = std::mem::replace(&mut self.owned_receiver, false);
        let result = walk_bind(self, bind);
        self.owned_receiver = previous;
        result
    }

    fn visit_when_expr(&mut self, expression: &ast::WhenExpr) -> ControlFlow<()> {
        for arm in &expression.arms {
            let body = match arm {
                ast::WhenArm::Cond { body, .. }
                | ast::WhenArm::Is { body, .. }
                | ast::WhenArm::Else(body, _) => body,
            };
            if let Expr::Block(expressions) = &body.value {
                self.sequence(
                    expressions,
                    self.context.parsed.ast.span_table.get(body.span_id).end(),
                );
            }
        }
        ast::folder::walk_when(self, expression)
    }

    fn visit_bind_value(&mut self, value: &BindValue) -> ControlFlow<()> {
        if let BindValue::Body { exprs, ret } = value {
            self.sequence(exprs, body_end(self.context, exprs, ret));
        }
        walk_bind_value(self, value)
    }

    fn visit_if_expr(&mut self, expression: &ast::IfExpr) -> ControlFlow<()> {
        self.sequence(
            &expression.body,
            body_end(self.context, &expression.body, &expression.ret),
        );
        walk_if(self, expression)
    }

    fn visit_while_loop(&mut self, expression: &ast::WhileLoop) -> ControlFlow<()> {
        let end = expression
            .exprs
            .iter()
            .map(|expr| self.context.parsed.ast.span_table.get(expr.span_id).end())
            .max()
            .unwrap_or_default();
        self.sequence(&expression.exprs, end);
        walk_while(self, expression)
    }

    fn visit_for_in_loop(&mut self, expression: &ast::ForInLoop) -> ControlFlow<()> {
        let end = expression
            .exprs
            .iter()
            .map(|expr| self.context.parsed.ast.span_table.get(expr.span_id).end())
            .max()
            .unwrap_or_default();
        self.sequence(&expression.exprs, end);
        walk_for_in(self, expression)
    }
}

#[derive(Default)]
struct ReceiverShadow {
    found: bool,
}

impl Visitor for ReceiverShadow {
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        self.found |= bind.name.as_str() == "self";
        if bind.params.is_some() {
            ControlFlow::Continue(())
        } else {
            walk_bind(self, bind)
        }
    }

    fn visit_condition(&mut self, condition: &ast::Condition) -> ControlFlow<()> {
        if let ast::Condition::Is { pattern, .. } = condition {
            self.found |= pattern
                .value
                .binding_names()
                .iter()
                .any(|name| name.as_str() == "self");
        }
        walk_condition(self, condition)
    }

    fn visit_expr(&mut self, expression: &Expr) -> ControlFlow<()> {
        let binds_self = |pattern: &ast::Pattern| {
            pattern
                .binding_names()
                .iter()
                .any(|name| name.as_str() == "self")
        };
        match expression {
            Expr::Destructure { field_bindings, .. } => {
                self.found |= field_bindings
                    .iter()
                    .any(|(_, name)| name.as_str() == "self");
            }
            Expr::When(expression) => {
                self.found |= expression.arms.iter().any(|arm| {
                    matches!(arm, ast::WhenArm::Is { pattern, .. } if binds_self(&pattern.value))
                });
            }
            Expr::Loop(ast::Loop::ForIn(expression)) => {
                self.found |= binds_self(&ast::Pattern::from_expr(expression.pat.value.clone()));
            }
            _ => {}
        }
        walk_expr(self, expression)
    }
}

fn body_end(context: &HelpContext<'_>, expressions: &[Typed<Expr>], ret: &ast::Return) -> usize {
    let spans = &context.parsed.ast.span_table;
    expressions
        .iter()
        .chain(&ret.values)
        .map(|expr| spans.get(expr.span_id).end())
        .chain([spans.get(ret.span_id).end()])
        .max()
        .unwrap_or_default()
}

fn pattern_edit(
    source: &str,
    pattern: Span,
    field: Intern<String>,
    binding: Intern<String>,
    alias: Intern<String>,
) -> Option<Edit> {
    let mut lexer = Lexer::new(&source[pattern.to_range()]);
    let mut tokens = Vec::new();
    while let Some((token, span)) = lexer.next_raw() {
        if matches!(
            token,
            Token::Id(_) | Token::Colon | Token::Comma | Token::ParenOpen | Token::ParenClose
        ) {
            tokens.push((token, lexer.get_span(span)));
        }
    }
    let start = tokens
        .iter()
        .position(|(token, _)| *token == Token::ParenOpen)?
        + 1;
    for entry in
        tokens[start..].split(|(token, _)| matches!(token, Token::Comma | Token::ParenClose))
    {
        let (span, replacement) = match entry {
            [(Token::Id(name), span)] if *name == field.as_str() && field == binding => {
                (*span, format!("{field}: {alias}"))
            }
            [
                (Token::Id(name), _),
                (Token::Colon, _),
                (Token::Id(name_binding), span),
            ] if *name == field.as_str() && *name_binding == binding.as_str() => {
                (*span, alias.to_string())
            }
            _ => continue,
        };
        return Some(Edit {
            span: Span::new(pattern.start() + span.start(), pattern.start() + span.end()),
            replacement,
        });
    }
    None
}

fn alias_removal(source: &str, alias: Span) -> Option<Edit> {
    let start = source[..alias.start()]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    if !source[start..alias.start()]
        .chars()
        .all(char::is_whitespace)
    {
        return None;
    }
    let end = source[alias.end()..]
        .find('\n')
        .map_or(source.len(), |newline| alias.end() + newline + 1);
    let suffix = &source[alias.end()..end];
    let span = if suffix.trim().is_empty() {
        Span::new(start, end)
    } else if suffix.trim_start().starts_with("--") {
        Span::new(
            alias.start(),
            alias.end() + suffix.len() - suffix.trim_start().len(),
        )
    } else {
        return None;
    };
    Some(Edit {
        span,
        replacement: String::new(),
    })
}

fn plain_alias(expression: &Expr) -> Option<(&Bind, Intern<String>)> {
    let Expr::Bind(bind) = expression else {
        return None;
    };
    if bind.operator != BindOperator::FreshMutable
        || bind.params.is_some()
        || bind.return_tag.is_some()
        || bind.return_type_name.is_some()
        || bind.type_annotation.is_some()
        || bind.return_refinement.is_some()
        || bind.receiver_type.is_some()
        || !bind.named_results.is_empty()
        || bind.doc_comment.is_some()
        || bind.attributes.raw_attributes.is_some()
    {
        return None;
    }
    let BindValue::Expr(value) = &bind.value else {
        return None;
    };
    let Expr::FnCall(call) = &value.value else {
        return None;
    };
    (call.args.is_none() && call.path.is_bare()).then_some((bind, call.path.root))
}
