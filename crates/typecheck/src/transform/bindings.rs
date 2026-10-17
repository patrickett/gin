use diagnostic::code::BindingCode;
use std::collections::HashMap;
use std::ops::ControlFlow;

use ast::prelude::*;
use diagnostic::{Diagnostic, RelatedSpan};

use crate::TypedFileAst;

pub(super) fn validate(file: &FileAst, typed: &mut TypedFileAst) {
    let mut bindings = Bindings {
        typed,
        visible: HashMap::new(),
        introduced: Vec::new(),
    };
    let mut definitions: Vec<_> = file.defs.values().collect();
    definitions.sort_by_key(|bind| file.span_table.get(bind.name_span).start());
    for bind in definitions {
        bindings.scoped(|bindings| bindings.callable(bind));
    }
    for (expression, span) in &file.exprs {
        bindings.expression(expression, *span);
    }
}

struct Bindings<'a> {
    typed: &'a mut TypedFileAst,
    visible: HashMap<Intern<String>, SpanId>,
    introduced: Vec<Intern<String>>,
}

impl Bindings<'_> {
    fn declare(&mut self, name: Intern<String>, span: SpanId) {
        if name.as_str() == "_" {
            return;
        }
        if let Some(previous) = self.visible.get(&name) {
            // type-binding-shadows-visible-name
            let mut diagnostic = Diagnostic::new(
                BindingCode::ShadowsVisibleName,
                format!("declaration `{name}` hides a visible local or parameter"),
            )
            .with_arg("name", name.as_str())
            .with_help("choose a different name; use `::` to update an existing mutable binding")
            .at_span_id(span, &self.typed.span_table);
            diagnostic.related_mut().push(RelatedSpan {
                span: self.typed.span_table.get(*previous),
                label: "previous declaration is still visible".to_string(),
            });
            self.typed.declaration_flaws.push((span, diagnostic));
            return;
        }
        self.visible.insert(name, span);
        self.introduced.push(name);
    }

    fn restore(&mut self, checkpoint: usize) {
        for name in self.introduced.drain(checkpoint..) {
            assert!(self.visible.remove(&name).is_some());
        }
    }

    fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
        let checkpoint = self.introduced.len();
        visit(self);
        self.restore(checkpoint);
    }

    fn callable(&mut self, bind: &Bind) {
        for (name, parameter) in bind.params.iter().flat_map(|params| params.iter()) {
            if let ParameterKind::Default(value) = &parameter.kind {
                self.typed_expression(value);
            }
            self.declare(*name, parameter.name_span);
        }
        self.value(&bind.value);
    }

    fn value(&mut self, value: &BindValue) {
        match value {
            BindValue::Expr(expression) => self.typed_expression(expression),
            BindValue::Body { exprs, ret } => {
                self.expressions(exprs);
                self.expressions(&ret.values);
            }
            BindValue::Unassigned => {}
        }
    }

    fn expressions(&mut self, expressions: &[Typed<Expr>]) {
        for expression in expressions {
            self.typed_expression(expression);
        }
    }

    fn initializer(&mut self, value: &BindValue) {
        match value {
            BindValue::Body { .. } => self.scoped(|bindings| bindings.value(value)),
            _ => self.value(value),
        }
    }

    fn typed_expression(&mut self, expression: &Typed<Expr>) {
        self.expression(&expression.value, expression.span_id);
    }

    fn expression(&mut self, expression: &Expr, span: SpanId) {
        match expression {
            Expr::FnCall(call) => self.arguments(call.args.as_deref().unwrap_or_default()),
            Expr::TagCall(call) => self.arguments(&call.args),
            Expr::Bind(bind) if bind.is_rebind() => self.initializer(&bind.value),
            Expr::Bind(bind) if bind.params.is_some() => {
                self.declare(bind.name, bind.name_span);
                self.scoped(|bindings| bindings.callable(bind));
            }
            Expr::Bind(bind) => {
                self.initializer(&bind.value);
                self.declare(bind.name, bind.name_span);
            }
            Expr::Destructure {
                field_bindings,
                value,
                ..
            } => {
                self.typed_expression(value);
                for (_, name) in field_bindings {
                    self.declare(*name, span);
                }
            }
            Expr::Block(expressions) => {
                self.scoped(|bindings| bindings.expressions(expressions));
            }
            Expr::If(branch) => self.scoped(|bindings| {
                bindings.condition(&branch.condition);
                bindings.expressions(&branch.body);
                bindings.expressions(&branch.ret.values);
            }),
            Expr::When(when) => {
                if let Some(subject) = &when.subject {
                    self.typed_expression(subject);
                }
                for arm in &when.arms {
                    self.scoped(|bindings| bindings.arm(arm));
                }
            }
            Expr::Loop(ast::Loop::While(while_loop)) => self.scoped(|bindings| {
                bindings.condition(&while_loop.condition);
                bindings.expressions(&while_loop.exprs);
            }),
            Expr::Loop(ast::Loop::ForIn(for_loop)) => {
                self.typed_expression(&for_loop.iter);
                self.scoped(|bindings| {
                    match &for_loop.pat.value {
                        Expr::Bind(bind) => bindings.declare(bind.name, bind.name_span),
                        expression => {
                            for name in expression.for_loop_pattern_names().unwrap_or_default() {
                                bindings.declare(name, for_loop.pat.span_id);
                            }
                        }
                    }
                    bindings.expressions(&for_loop.exprs);
                });
            }
            _ => {
                let _ = ast::folder::walk_typed_expr_children(expression, &mut |child| {
                    self.typed_expression(child);
                    ControlFlow::Continue(())
                });
            }
        }
    }

    fn arguments(&mut self, arguments: &[Typed<Expr>]) {
        for argument in arguments {
            match &argument.value {
                Expr::Bind(bind)
                    if bind.params.is_none() && matches!(bind.value, BindValue::Expr(_)) =>
                {
                    self.value(&bind.value);
                }
                _ => self.typed_expression(argument),
            }
        }
    }

    fn arm(&mut self, arm: &WhenArm) {
        match arm {
            WhenArm::Cond {
                condition, body, ..
            } => {
                self.condition(condition);
                self.typed_expression(body);
            }
            WhenArm::Is { pattern, body, .. } => {
                self.pattern(&pattern.value, pattern.span_id);
                self.typed_expression(body);
            }
            WhenArm::Else(body, _) => self.typed_expression(body),
        }
    }

    fn pattern(&mut self, pattern: &Pattern, span: SpanId) {
        match pattern {
            Pattern::Generic {
                params,
                param_spans,
                ..
            } => {
                for ((_, kind), (name, span)) in params.iter().zip(param_spans) {
                    if matches!(kind, ParameterKind::Default(_))
                        || !name.chars().next().is_some_and(|character| {
                            character.is_ascii_lowercase() || character == '_'
                        })
                    {
                        continue;
                    }
                    self.declare(*name, *span);
                }
            }
            Pattern::ListCons { head, tail } => {
                self.pattern(&head.value, head.span_id);
                self.pattern(&tail.value, tail.span_id);
            }
            _ => {
                for name in pattern.binding_names() {
                    self.declare(name, span);
                }
            }
        }
    }

    fn condition(&mut self, condition: &ast::Condition) {
        match condition {
            ast::Condition::Is { subject, pattern } => {
                self.typed_expression(subject);
                self.pattern(&pattern.value, pattern.span_id);
            }
            ast::Condition::And(left, right) => {
                self.condition(left);
                self.condition(right);
            }
            ast::Condition::Or(left, right) => self.alternatives(left, right),
            ast::Condition::Not(inner) => self.scoped(|bindings| bindings.condition(inner)),
            _ => {
                let _ = ast::folder::walk_condition_typed_exprs(condition, &mut |child| {
                    self.typed_expression(child);
                    ControlFlow::Continue(())
                });
            }
        }
    }

    fn alternatives(&mut self, left: &ast::Condition, right: &ast::Condition) {
        let checkpoint = self.introduced.len();
        self.condition(left);
        let left_bindings: Vec<_> = self.introduced[checkpoint..]
            .iter()
            .map(|name| (*name, self.visible[name]))
            .collect();
        self.restore(checkpoint);
        self.condition(right);
        let common: Vec<_> = left_bindings
            .into_iter()
            .filter(|(name, _)| self.visible.contains_key(name))
            .collect();
        self.restore(checkpoint);
        for (name, span) in common {
            self.declare(name, span);
        }
    }
}
