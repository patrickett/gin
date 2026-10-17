use diagnostic::code::FormatCode;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use ast::folder::{Visitor, walk_bind, walk_typed_expr_children};
use ast::{
    Bind, BindValue, Declare, DeclareValue, Expr, HasMember, HasMemberBody, ParameterKind,
    Parameters, SpanId, Typed,
};
use diagnostic::{Category, Diagnostic, Span};
use internment::Intern;
use lexer::{Lexer, Token};

use super::{Edit, Help, HelpContext};

impl HelpContext<'_> {
    pub(super) fn generic_name_suggestions(&self) -> Vec<Help> {
        let mut visitor = GenericNames {
            context: self,
            bound: HashSet::new(),
            helps: Vec::new(),
        };
        let _ = visitor.visit_file_ast(&self.parsed.ast);
        for declaration in self.parsed.ast.tags.values() {
            visitor.declaration(declaration);
        }
        visitor.helps
    }
}

struct GenericNames<'a, 'b> {
    context: &'a HelpContext<'b>,
    bound: HashSet<Intern<String>>,
    helps: Vec<Help>,
}

impl GenericNames<'_, '_> {
    fn declaration(&mut self, declaration: &Declare) {
        let previous = self.bound.clone();
        self.bound.extend(
            declaration
                .params
                .iter()
                .flat_map(|params| params.keys())
                .copied(),
        );
        if let DeclareValue::Has(members) = &declaration.value {
            for member in members {
                let HasMember::Function(function) = member else {
                    continue;
                };
                let body = function.body.as_ref().map(|body| match body {
                    HasMemberBody::Overrideable(body) | HasMemberBody::Final(body) => body,
                });
                let surfaces = function
                    .return_ty
                    .iter()
                    .map(|ty| ty.span_id)
                    .chain(
                        function
                            .anonymous_result_alternatives
                            .iter()
                            .map(|result| result.span_id),
                    )
                    .collect::<Vec<_>>();
                self.parameters(&function.params, body, &surfaces);
                let method_scope = self.bound.clone();
                self.bind_parameters(&function.params);
                if let Some(body) = body {
                    let _ = self.visit_bind_value(body);
                }
                self.bound = method_scope;
            }
        }
        for extension in &declaration.extensions {
            self.declaration(extension);
        }
        self.bound = previous;
    }

    fn bind_parameters(&mut self, params: &Parameters) {
        self.bound.extend(params.keys().copied());
        for parameter in params.values() {
            if let ParameterKind::Tagged(surface) = &parameter.kind {
                let span = self.context.parsed.ast.span_table.get(surface.span_id);
                self.bound
                    .extend(names(&self.context.source[span.to_range()]).into_keys());
            }
        }
    }

    fn parameters(&mut self, params: &Parameters, body: Option<&BindValue>, surfaces: &[SpanId]) {
        let spans = &self.context.parsed.ast.span_table;
        let Some(start) = params
            .values()
            .map(|parameter| spans.get(parameter.name_span).start())
            .min()
        else {
            return;
        };
        let mut end = start;
        let mut include = |id| {
            end = end.max(spans.get(id).end());
        };
        for parameter in params.values() {
            include(parameter.name_span);
            match &parameter.kind {
                ParameterKind::Tagged(surface)
                | ParameterKind::ValueParam { ty: surface }
                | ParameterKind::Inferred { ty: surface } => include(surface.span_id),
                ParameterKind::Default(value) => include(value.span_id),
                ParameterKind::Generic | ParameterKind::Proposition(_) => {}
            }
            for constraint in &parameter.constraints {
                include(constraint.trait_name_span);
                for arg in &constraint.args {
                    include(arg.span_id);
                }
            }
        }
        for span in surfaces {
            include(*span);
        }
        let mut expression = |expr: &Typed<Expr>| {
            include(expr.span_id);
            let _ = walk_typed_expr_children(expr, &mut |child| {
                include(child.span_id);
                ControlFlow::<()>::Continue(())
            });
        };
        match body {
            Some(BindValue::Expr(expr)) => expression(expr),
            Some(BindValue::Body { exprs, ret }) => {
                for expr in exprs.iter().chain(&ret.values) {
                    expression(expr);
                }
                include(ret.span_id);
            }
            _ => {}
        }
        let occurrences = names(&self.context.source[start..end]);
        for (parameter_name, parameter) in params {
            let ParameterKind::Tagged(surface) = &parameter.kind else {
                continue;
            };
            let (ty, span) = match &surface.value {
                Expr::Ref { inner, .. } => (&inner.value, spans.get(inner.span_id)),
                ty => (ty, spans.get(surface.span_id)),
            };
            let name = match ty {
                Expr::AnonymousTag(name) => *name,
                Expr::FnCall(call) if call.args.is_none() && call.path.is_bare() => call.path.root,
                _ => continue,
            };
            if !name.chars().next().is_some_and(char::is_lowercase)
                || occurrences.get(&name) != Some(&1)
                || self.bound.contains(&name)
                || params.contains_key(&name)
                || self.context.package.iter().any(|file| {
                    file.tag_types.keys().any(|id| id.0 == name)
                        || file.defs.keys().any(|id| id.0 == name)
                })
            {
                continue;
            }
            if self.context.source.get(span.to_range()) != Some(name.as_str()) {
                continue;
            }
            let parameter_end = spans.get(parameter.name_span).end();
            let gap = &self.context.source[parameter_end..span.start()];
            if gap.contains(['\n', '\r']) || gap.contains("--") {
                continue;
            }
            let mut remove_start = span.start();
            while remove_start > parameter_end
                && matches!(
                    self.context.source.as_bytes()[remove_start - 1],
                    b' ' | b'\t'
                )
            {
                remove_start -= 1;
            }
            self.helps.push(Help {
                // fmt-unused-generic-type-name
                diagnostic: Diagnostic::new(
                    FormatCode::UnusedGenericTypeName,
                    format!(
                        "generic type name `{name}` is only used by parameter `{parameter_name}`"
                    ),
                )
                .with_category(Category::Help)
                .with_arg("type_parameter", name.to_string())
                .with_help(format!("Remove unused generic type name `{name}`"))
                .at_span(span),
                edits: vec![Edit {
                    span: Span::new(remove_start, span.end()),
                    replacement: String::new(),
                }],
            });
        }
    }
}

impl Visitor for GenericNames<'_, '_> {
    fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
        if bind.is_method() {
            return ControlFlow::Continue(());
        }
        let previous = self.bound.clone();
        if let Some(params) = &bind.params {
            let surfaces = bind
                .return_tag
                .iter()
                .map(|ty| ty.span_id)
                .chain(bind.named_results.iter().map(|result| result.ty.span_id))
                .chain(
                    bind.anonymous_result_alternatives
                        .iter()
                        .map(|result| result.span_id),
                )
                .collect::<Vec<_>>();
            self.parameters(params, Some(&bind.value), &surfaces);
            self.bind_parameters(params);
        }
        let result = walk_bind(self, bind);
        self.bound = previous;
        result
    }
}

fn names(source: &str) -> HashMap<Intern<String>, usize> {
    let mut names = HashMap::new();
    for (token, _) in Lexer::new(source) {
        if let Token::Id(name) = token {
            *names.entry(Intern::from_ref(name)).or_default() += 1;
        }
    }
    names
}
