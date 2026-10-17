use diagnostic::code::FormatCode;
use std::collections::HashSet;
use std::ops::ControlFlow;

use ast::folder::{Visitor, walk_bind, walk_condition, walk_expr, walk_fn_call};
use ast::{
    Bind, BindValue, Declare, DeclareValue, Expr, FnCall, HasMember, HasMemberBody, ParameterKind,
    Parameters, SpanId,
};
use diagnostic::{Category, Diagnostic};
use internment::Intern;

use super::{Edit, Help, HelpContext};

impl HelpContext<'_> {
    pub(super) fn parameter_name_suggestions(&self) -> Vec<Help> {
        let mut visitor = ParameterNames {
            context: self,
            helps: Vec::new(),
        };
        let _ = visitor.visit_file_ast(&self.parsed.ast);
        for declaration in self.parsed.ast.tags.values() {
            visitor.declaration(declaration);
        }
        visitor.helps
    }
}

struct ParameterNames<'a, 'b> {
    context: &'a HelpContext<'b>,
    helps: Vec<Help>,
}

impl ParameterNames<'_, '_> {
    fn declaration(&mut self, declaration: &Declare) {
        if let DeclareValue::Has(members) = &declaration.value {
            for member in members {
                let HasMember::Function(function) = member else {
                    continue;
                };
                let body = function.body.as_ref().map(|body| match body {
                    HasMemberBody::Overrideable(body) | HasMemberBody::Final(body) => body,
                });
                let surfaces: Vec<_> = function.return_ty.iter().map(|ty| &ty.value).collect();
                self.parameters(&function.params, body, &surfaces);
            }
        }
        for extension in &declaration.extensions {
            self.declaration(extension);
        }
    }

    fn parameters(&mut self, params: &Parameters, body: Option<&BindValue>, surfaces: &[&Expr]) {
        let ast = &self.context.parsed.ast;
        let spans = &ast.span_table;
        let mut uses = Names::default();
        if let Some(body) = body {
            let _ = uses.visit_bind_value(body);
        }
        let mut signature = Names::default();
        for parameter in params.values() {
            match &parameter.kind {
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => {
                    let _ = signature.visit_expr(&ty.value);
                }
                ParameterKind::Default(value) => {
                    let _ = signature.visit_expr(&value.value);
                }
                ParameterKind::Generic | ParameterKind::Proposition(_) => {}
            }
        }
        for surface in surfaces {
            let _ = signature.visit_expr(surface);
        }
        let mut occupied = uses.occupied.clone();
        occupied.extend(&signature.occupied);
        occupied.extend(params.keys().copied());
        for (name, parameter) in params {
            let ty = match &parameter.kind {
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => &ty.value,
                ParameterKind::Generic
                | ParameterKind::Default(_)
                | ParameterKind::Proposition(_) => continue,
            };
            let Some(ty_name) = type_name(ty) else {
                continue;
            };
            if name.as_str() == "self"
                || !name.eq_ignore_ascii_case(ty_name.as_str())
                || uses.pattern_bindings.contains(name)
            {
                continue;
            }
            let suggested = (1..ty_name.chars().count())
                .map(|length| {
                    ty_name
                        .chars()
                        .take(length)
                        .collect::<String>()
                        .to_ascii_lowercase()
                })
                .find(|candidate| {
                    lexer::Token::from_keyword(candidate).is_none()
                        && !occupied.contains(&Intern::from_ref(candidate))
                });
            let Some(suggested) = suggested else { continue };
            occupied.insert(Intern::from_ref(&suggested));
            let declaration = spans.get(parameter.name_span);
            let mut edits = vec![Edit {
                span: declaration,
                replacement: suggested.clone(),
            }];
            for (reference, span) in &uses.references {
                let span = spans.get(*span);
                if reference == name
                    && resolve::local_definition_span(ast, name, span.start())
                        == Some(declaration.to_range())
                {
                    edits.push(Edit {
                        span,
                        replacement: suggested.clone(),
                    });
                }
            }
            for (reference, span) in &signature.references {
                if reference == name {
                    edits.push(Edit {
                        span: spans.get(*span),
                        replacement: suggested.clone(),
                    });
                }
            }
            edits.sort_by_key(|edit| edit.span.start());
            edits.dedup_by_key(|edit| edit.span);
            self.helps.push(Help {
                // fmt-duplicate-parameter-name
                diagnostic: Diagnostic::new(
                    FormatCode::DuplicateParameterName,
                    format!("parameter `{name}` repeats its type name; consider `{suggested}`"),
                )
                .with_category(Category::Help)
                .with_arg("parameter", name.to_string())
                .with_arg("suggestion", suggested.clone())
                .with_help(format!("Rename parameter `{name}` to `{suggested}`"))
                .at_span(declaration),
                edits,
            });
        }
    }
}

impl Visitor for ParameterNames<'_, '_> {
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        if let Some(params) = &bind.params {
            let surfaces: Vec<_> = bind
                .return_tag
                .iter()
                .map(|ty| &ty.value)
                .chain(bind.named_results.iter().map(|result| &result.ty.value))
                .collect();
            self.parameters(params, Some(&bind.value), &surfaces);
        }
        walk_bind(self, bind)
    }
}

#[derive(Default)]
struct Names {
    occupied: HashSet<Intern<String>>,
    pattern_bindings: HashSet<Intern<String>>,
    references: Vec<(Intern<String>, SpanId)>,
}

impl Visitor for Names {
    fn visit_condition(&mut self, condition: &ast::Condition) -> ControlFlow<()> {
        if let ast::Condition::Is { pattern, .. } = condition {
            let names = pattern.value.binding_names();
            self.occupied.extend(&names);
            self.pattern_bindings.extend(names);
        }
        walk_condition(self, condition)
    }

    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        self.occupied.insert(bind.name);
        self.occupied
            .extend(bind.params.iter().flat_map(|params| params.keys()).copied());
        if bind.is_rebind() {
            self.references.push((bind.name, bind.name_span));
        }
        walk_bind(self, bind)
    }

    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        self.occupied.insert(call.path.root);
        if call.path.root_span.is_valid() {
            self.references.push((call.path.root, call.path.root_span));
        } else if call.path.is_bare() {
            self.references.push((call.path.root, call.path.span_id));
        }
        walk_fn_call(self, call)
    }

    fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        let mut pattern = |pattern: &ast::Pattern| {
            let names = pattern.binding_names();
            self.occupied.extend(&names);
            self.pattern_bindings.extend(names);
        };
        match expr {
            Expr::When(when) => {
                for arm in &when.arms {
                    if let ast::WhenArm::Is { pattern: pat, .. } = arm {
                        pattern(&pat.value);
                    }
                }
            }
            Expr::Loop(ast::Loop::ForIn(loop_)) => {
                pattern(&ast::Pattern::from_expr(loop_.pat.value.clone()))
            }
            Expr::Destructure { field_bindings, .. } => {
                for (_, name) in field_bindings {
                    self.occupied.insert(*name);
                    self.pattern_bindings.insert(*name);
                }
            }
            _ => {}
        }
        walk_expr(self, expr)
    }
}

fn type_name(expr: &Expr) -> Option<Intern<String>> {
    match expr {
        Expr::AnonymousTag(name) => Some(*name),
        Expr::TagCall(call) => Some(call.name),
        Expr::FnCall(call) => Some(call.path.segments.last().copied().unwrap_or(call.path.root)),
        Expr::Ref { inner, .. } => type_name(&inner.value),
        _ => None,
    }
}
