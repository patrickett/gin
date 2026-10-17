use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;

use ast::folder::{Folder, walk_typed_expr_children_mut};
use ast::{Bind, Expr, ParameterKind, SpanTable};
use internment::Intern;

use super::super::proof_walk::write::{ProofVisitorMut, walk_normal, walk_reference, walk_term};
use super::Methods;
use super::arguments::{Arguments, argument_order};
use crate::typed::{DefId, TypedBind};

pub(super) fn specialize(
    bind: &Bind,
    name: Intern<String>,
    arguments: &Arguments,
    methods: &Methods,
) -> Bind {
    let Arguments { values, receivers } = arguments;
    let mut bind = bind.clone();
    bind.name = name;
    let inferred: BTreeSet<_> = bind
        .params
        .iter()
        .flat_map(|params| params.keys())
        .filter(|name| super::super::parameters::is_inferred_value(&bind, **name))
        .copied()
        .collect();
    if let Some(params) = &mut bind.params {
        for (name, parameter) in params
            .iter_mut()
            .filter(|(name, _)| inferred.contains(name))
        {
            let key = super::super::parameters::inferred_value_type_name(*name);
            if let Some(ty) = values.get(&key) {
                let mut surface = ty.clone();
                if let Some(
                    convention @ (ast::ParamConvention::Observe | ast::ParamConvention::Mutate),
                ) = bind.param_conventions.get(name)
                {
                    surface = Expr::Ref {
                        inner: Box::new(ast::Typed::infer(surface, parameter.name_span)),
                        mutable: *convention == ast::ParamConvention::Mutate,
                        group: bind.param_groups.get(name).cloned(),
                    };
                }
                parameter.kind = ParameterKind::Tagged(Box::new(ast::Spanned::new(
                    surface,
                    parameter.name_span,
                )));
            }
        }
        params.retain(|name, param| {
            !(values.contains_key(name) && matches!(param.kind, ParameterKind::Generic))
        });
    }
    let mut folder = SpecializeTypes {
        values,
        receivers,
        methods,
        shadowed: BTreeSet::new(),
    };
    let _ = folder.visit_bind(&mut bind);
    bind
}

pub(super) struct RewriteCall<'a> {
    pub(super) spans: &'a SpanTable,
    pub(super) span: ast::SpanId,
    pub(super) definition: DefId,
    pub(super) template: &'a TypedBind,
    pub(super) member_call: Option<ast::FnCall>,
}

impl RewriteCall<'_> {
    pub(super) fn rewrite_bind(&mut self, bind: &mut Bind) {
        match &mut bind.value {
            ast::BindValue::Expr(expr) => {
                let _ = self.rewrite_expr(expr);
            }
            ast::BindValue::Body { exprs, ret } => {
                for expr in exprs.iter_mut().chain(&mut ret.values) {
                    let _ = self.rewrite_expr(expr);
                }
            }
            ast::BindValue::Unassigned => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut ast::Typed<Expr>) -> ControlFlow<()> {
        if self.spans.get(expr.span_id) == self.spans.get(self.span)
            && let Expr::Binary(binary) = &expr.value
        {
            expr.value = Expr::FnCall(ast::FnCall {
                syntax: ast::CallSyntax::Path,
                definition: Some(self.definition),
                path: ast::Spanned::new(
                    ast::ModPath::new(self.definition.0, Vec::new()),
                    expr.span_id,
                ),
                args: Some(vec![(*binary.lhs).clone(), (*binary.rhs).clone()]),
            });
        }
        if self.spans.get(expr.span_id) == self.spans.get(self.span)
            && let Expr::RecordGet { field, .. } = &mut expr.value
        {
            *field = Intern::from_ref(self.definition.0.as_str().rsplit('.').next().unwrap());
        } else if let Expr::FnCall(call) = &mut expr.value
            && call.args.is_some()
            && if call.definition.is_some() {
                expr.span_id == self.span
            } else {
                self.spans.get(expr.span_id) == self.spans.get(self.span)
            }
        {
            if let Some(member) = &self.member_call {
                *call = member.clone();
            }
            if let Some(args) = &mut call.args
                && let Some(order) = argument_order(args, self.template)
            {
                let removed: BTreeSet<_> = order
                    .into_iter()
                    .enumerate()
                    .filter_map(|(parameter, argument)| {
                        matches!(self.template.param_kinds[parameter], ast::ParamKind::Type)
                            .then_some(argument)
                    })
                    .collect();
                let mut index = 0;
                args.retain(|_| {
                    let keep = !removed.contains(&index);
                    index += 1;
                    keep
                });
            }
            call.definition = Some(self.definition);
            call.path.value.root = self.definition.0;
            call.path.value.segments.clear();
            call.path.value.segment_spans.clear();
        }
        walk_typed_expr_children_mut(&mut expr.value, &mut |child| self.rewrite_expr(child))
    }
}

struct SpecializeTypes<'a> {
    values: &'a BTreeMap<Intern<String>, Expr>,
    receivers: &'a BTreeMap<Intern<String>, ast::ty::TypeId>,
    methods: &'a Methods,
    shadowed: BTreeSet<Intern<String>>,
}

impl SpecializeTypes<'_> {
    fn with_names(
        &mut self,
        names: Vec<Intern<String>>,
        visit: impl FnOnce(&mut Self) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let introduced: Vec<_> = names
            .into_iter()
            .filter(|name| self.shadowed.insert(*name))
            .collect();
        let result = visit(self);
        for name in introduced {
            self.shadowed.remove(&name);
        }
        result
    }

    fn condition_names(condition: &ast::Condition) -> Vec<Intern<String>> {
        let mut pending = vec![condition];
        let mut names = Vec::new();
        while let Some(condition) = pending.pop() {
            match condition {
                ast::Condition::Is { pattern, .. } => names.extend(pattern.value.binding_names()),
                ast::Condition::And(left, right) => {
                    pending.push(right);
                    pending.push(left);
                }
                ast::Condition::Or(left, right) => {
                    let right = Self::condition_names(right);
                    names.extend(
                        Self::condition_names(left)
                            .into_iter()
                            .filter(|name| right.contains(name)),
                    );
                }
                ast::Condition::Proposition { .. }
                | ast::Condition::Compare { .. }
                | ast::Condition::Not(_) => {}
            }
        }
        names
    }

    fn statements(
        &mut self,
        exprs: &mut [ast::Typed<Expr>],
        ret: Option<&mut ast::Return>,
        names: Vec<Intern<String>>,
    ) -> ControlFlow<()> {
        let mut introduced: Vec<_> = names
            .into_iter()
            .filter(|name| self.shadowed.insert(*name))
            .collect();
        let result = (|| {
            for expr in exprs {
                self.visit_expr(&mut expr.value)?;
                let (name, fields) = match &expr.value {
                    Expr::Bind(bind) => (Some(bind.name), &[][..]),
                    Expr::Destructure { field_bindings, .. } => (None, field_bindings.as_slice()),
                    _ => (None, &[][..]),
                };
                for name in name.into_iter().chain(fields.iter().map(|(_, name)| *name)) {
                    if self.shadowed.insert(name) {
                        introduced.push(name);
                    }
                }
            }
            match ret {
                Some(ret) => self.visit_return(ret),
                None => ControlFlow::Continue(()),
            }
        })();
        for name in introduced {
            self.shadowed.remove(&name);
        }
        result
    }

    fn value(&self, name: &Intern<String>) -> Option<&Expr> {
        if self.shadowed.contains(name) {
            None
        } else {
            self.values.get(name)
        }
    }

    fn specialize_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        for predicate in bind.param_refinements.values_mut() {
            let _ = self.visit_predicate(predicate);
        }
        if let Some(predicate) = &mut bind.return_refinement {
            let _ = self.visit_predicate(predicate);
        }
        for proof in &mut bind.payload_postconditions {
            let introduced = self.shadowed.insert(proof.name);
            let _ = self.visit_proposition(&mut proof.proposition);
            if introduced {
                self.shadowed.remove(&proof.name);
            }
            let _ = self.visit_expr(&mut proof.ty.value);
        }
        for alternative in &mut bind.anonymous_result_alternatives {
            if let Some(proposition) = &mut alternative.value.proposition {
                let _ = self.visit_proposition(proposition);
            }
        }
        if let Some(value) = bind.return_type_name.and_then(|name| self.value(&name)) {
            bind.return_tag = Some(Box::new(ast::Spanned::new(value.clone(), bind.name_span)));
            bind.return_type_name = None;
        }
        if let Some(receiver) = &mut bind.receiver_type {
            let _ = self.visit_expr(&mut receiver.value);
        }
        for param in bind
            .params
            .iter_mut()
            .flat_map(|params| params.values_mut())
        {
            match &mut param.kind {
                ParameterKind::Proposition(predicate) => {
                    let _ = self.visit_proposition(predicate);
                }
                ParameterKind::Tagged(surface)
                | ParameterKind::ValueParam { ty: surface }
                | ParameterKind::Inferred { ty: surface } => {
                    let _ = self.visit_expr(&mut surface.value);
                }
                _ => {}
            }
            for constraint in &mut param.constraints {
                for arg in &mut constraint.args {
                    self.visit_expr(&mut arg.value)?;
                }
            }
        }
        if let Some(surface) = &mut bind.return_tag {
            let _ = self.visit_expr(&mut surface.value);
        }
        for result in &mut bind.named_results {
            let _ = self.visit_expr(&mut result.ty.value);
        }
        if let Some((_, args)) = &mut bind.type_annotation {
            for arg in args {
                self.visit_expr(&mut arg.value)?;
            }
        }
        if bind.params.is_some() {
            self.with_names(vec![bind.name], |folder| {
                ast::folder::walk_bind_mut(folder, bind)
            })
        } else {
            ast::folder::walk_bind_mut(self, bind)
        }
    }

    fn type_reference(expr: &Expr) -> Option<ast::TypeReference> {
        Some(match expr {
            Expr::AnonymousTag(name) => ast::TypeReference::Nominal(*name),
            Expr::TagCall(call) => ast::TypeReference::Application {
                name: call.name,
                arguments: call
                    .args
                    .iter()
                    .map(|arg| Self::type_reference(&arg.value))
                    .collect::<Option<Vec<_>>>()?,
            },
            Expr::TakePtr(inner) => {
                ast::TypeReference::Pointer(Box::new(Self::type_reference(&inner.value)?))
            }
            _ => return None,
        })
    }
}

impl ProofVisitorMut for SpecializeTypes<'_> {
    fn visit_normal(&mut self, expr: &mut ast::NormalExpr) -> ControlFlow<()> {
        if let ast::NormalExpr::Var(name) = expr
            && let Some(Expr::Lit(ast::Literal::Int(value))) = self.value(name)
        {
            *expr = ast::NormalExpr::from(*value);
        }
        walk_normal(self, expr)
    }

    fn visit_term(&mut self, term: &mut ast::ProofTerm) -> ControlFlow<()> {
        if let ast::ProofTerm::Name(name) = term
            && let Some(Expr::Lit(ast::Literal::Int(value))) = self.value(name)
        {
            *term = ast::ProofTerm::Value(*value);
        }
        walk_term(self, term)
    }

    fn visit_reference(&mut self, ty: &mut ast::TypeReference) -> ControlFlow<()> {
        if let ast::TypeReference::Nominal(name) = ty
            && let Some(value) = self.value(name).and_then(Self::type_reference)
        {
            *ty = value;
            return ControlFlow::Continue(());
        }
        walk_reference(self, ty)
    }
}

impl Folder for SpecializeTypes<'_> {
    fn visit_condition(&mut self, condition: &mut ast::Condition) -> ControlFlow<()> {
        if let ast::Condition::And(left, right) = condition {
            self.visit_condition(left)?;
            return self.with_names(Self::condition_names(left), |folder| {
                folder.visit_condition(right)
            });
        }
        ast::folder::walk_condition_mut(self, condition)
    }
    fn visit_bind_value(&mut self, value: &mut ast::BindValue) -> ControlFlow<()> {
        match value {
            ast::BindValue::Body { exprs, ret } => self.statements(exprs, Some(ret), Vec::new()),
            _ => ast::folder::walk_bind_value_mut(self, value),
        }
    }

    fn visit_if_expr(&mut self, branch: &mut ast::IfExpr) -> ControlFlow<()> {
        self.visit_condition(&mut branch.condition)?;
        let names = Self::condition_names(&branch.condition);
        self.statements(&mut branch.body, Some(&mut branch.ret), names)
    }

    fn visit_while_loop(&mut self, loop_: &mut ast::WhileLoop) -> ControlFlow<()> {
        self.visit_condition(&mut loop_.condition)?;
        let names = Self::condition_names(&loop_.condition);
        self.statements(&mut loop_.exprs, None, names)
    }

    fn visit_for_in_loop(&mut self, loop_: &mut ast::ForInLoop) -> ControlFlow<()> {
        self.visit_expr(&mut loop_.iter.value)?;
        let names = loop_.pat.value.for_loop_pattern_names().unwrap_or_default();
        self.statements(&mut loop_.exprs, None, names)
    }

    fn visit_return(&mut self, ret: &mut ast::Return) -> ControlFlow<()> {
        if let Some(proof) = &mut ret.postcondition {
            let _ = self.visit_proposition(&mut proof.value);
        }
        ast::folder::walk_return_mut(self, ret)
    }

    fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        let names = bind
            .params
            .iter()
            .flat_map(|params| params.keys())
            .copied()
            .collect();
        self.with_names(names, |folder| folder.specialize_bind(bind))
    }

    fn visit_when_arm(&mut self, arm: &mut ast::WhenArm) -> ControlFlow<()> {
        match arm {
            ast::WhenArm::Is { pattern, body, .. } => self
                .with_names(pattern.value.binding_names(), |folder| {
                    folder.visit_expr(&mut body.value)
                }),
            ast::WhenArm::Cond {
                condition, body, ..
            } => {
                self.visit_condition(condition)?;
                self.with_names(Self::condition_names(condition), |folder| {
                    folder.visit_expr(&mut body.value)
                })
            }
            ast::WhenArm::Else(body, _) => self.visit_expr(&mut body.value),
        }
    }

    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        match expr {
            Expr::Block(items) => return self.statements(items, None, Vec::new()),
            Expr::AnonymousTag(name) => {
                if let Some(value) = self.value(name) {
                    *expr = value.clone();
                    return ControlFlow::Continue(());
                }
            }
            Expr::FnCall(call) => {
                if let Some(value) = self.value(&call.path.root) {
                    if call.args.is_none() && call.path.segments.is_empty() {
                        *expr = value.clone();
                        return ControlFlow::Continue(());
                    }
                    if let Some(receiver) = self.receivers.get(&call.path.root)
                        && let [member] = call.path.segments.as_slice()
                        && let Some(symbol) = self.methods.get(&(*receiver, *member))
                    {
                        call.path.value.root = *symbol;
                        call.path.value.segments.clear();
                        call.path.value.segment_spans.clear();
                        call.definition = None;
                        return ast::folder::walk_expr_mut(self, expr);
                    }
                    if let Some(name) = Self::type_reference(value).and_then(|ty| ty.head_name()) {
                        call.path.value.root = name;
                        call.definition = None;
                    }
                }
            }
            Expr::Cast { ty, .. } | Expr::TargetQuery { operand: ty, .. } => {
                let _ = self.visit_reference(ty);
            }
            _ => {}
        }
        ast::folder::walk_expr_mut(self, expr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::folder::Visitor;
    use parser::query::{ParseOutput, SourceParseExt};

    fn outer_bind(parsed: &ParseOutput) -> Bind {
        parsed
            .ast
            .defs
            .values()
            .find(|bind| bind.name.as_str() == "outer")
            .expect("test source declares `outer`")
            .clone()
    }

    fn specialize_outer(parsed: &ParseOutput) -> Bind {
        specialize(
            &outer_bind(parsed),
            Intern::from_ref("outer$specialized"),
            &Arguments {
                values: BTreeMap::from([(
                    Intern::from_ref("element"),
                    Expr::AnonymousTag(Intern::from_ref("Word")),
                )]),
                receivers: BTreeMap::new(),
            },
            &Methods::new(),
        )
    }

    #[test]
    fn specialization_preserves_destructured_binding_scope() {
        let parsed = "outer(element, value element):\n    Holder(field: element) := element(1)\n    return element\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let specialized = specialize_outer(&parsed);
        let ast::BindValue::Body { exprs, ret } = &specialized.value else {
            panic!("expected body")
        };
        let Expr::Destructure {
            field_bindings,
            value,
            ..
        } = &exprs[0].value
        else {
            panic!("expected destructuring")
        };
        assert_eq!(
            field_bindings,
            &vec![(Intern::from_ref("field"), Intern::from_ref("element"))]
        );
        let Expr::FnCall(initializer) = &value.value else {
            panic!("expected specialized initializer")
        };
        assert_eq!(initializer.path.root.as_str(), "Word");
        let Expr::FnCall(local) = &ret.values[0].value else {
            panic!("expected destructured local lookup")
        };
        assert_eq!(local.path.root.as_str(), "element");
    }

    #[test]
    fn specialization_preserves_recursive_callable_binding_scope() {
        let parsed = "outer(element, value element):\n    element(item element) element: return element(item)\n    return element(value)\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let specialized = specialize_outer(&parsed);
        let ast::BindValue::Body { exprs, ret } = &specialized.value else {
            panic!("expected body")
        };
        let Expr::Bind(local) = &exprs[0].value else {
            panic!("expected local callable")
        };
        let ParameterKind::Tagged(parameter) =
            &local.params.as_ref().unwrap()[&Intern::from_ref("item")].kind
        else {
            panic!("expected concrete parameter")
        };
        assert_eq!(
            parameter.value,
            Expr::AnonymousTag(Intern::from_ref("Word"))
        );
        assert_eq!(
            local.return_tag.as_ref().unwrap().value,
            Expr::AnonymousTag(Intern::from_ref("Word"))
        );
        let ast::BindValue::Body {
            ret: local_return, ..
        } = &local.value
        else {
            panic!("expected callable body")
        };
        let Expr::FnCall(recursive) = &local_return.values[0].value else {
            panic!("expected recursive call")
        };
        assert_eq!(recursive.path.root.as_str(), "element");
        let Expr::FnCall(after) = &ret.values[0].value else {
            panic!("expected local callable use")
        };
        assert_eq!(after.path.root.as_str(), "element");
    }

    #[test]
    fn specialization_preserves_when_pattern_scope() {
        let parsed = "outer(element, value element): return when value is Some(element) then element else element(1)\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let original = outer_bind(&parsed);
        let mut conditional = original.clone();
        let ast::BindValue::Body { ret, .. } = &mut conditional.value else {
            panic!("expected when expression");
        };
        let Expr::When(when) = &mut ret.values[0].value else {
            panic!("expected when expression");
        };
        let ast::WhenArm::Is {
            pattern,
            body,
            arm_span,
        } = when.arms[0].clone()
        else {
            panic!("expected pattern arm");
        };
        when.arms[0] = ast::WhenArm::Cond {
            condition: ast::Condition::Is {
                subject: when.subject.take().unwrap(),
                pattern,
            },
            body,
            arm_span,
        };
        for original in [&original, &conditional] {
            let specialized = specialize(
                original,
                Intern::from_ref("outer$specialized"),
                &Arguments {
                    values: BTreeMap::from([(
                        Intern::from_ref("element"),
                        Expr::AnonymousTag(Intern::from_ref("Word")),
                    )]),
                    receivers: BTreeMap::new(),
                },
                &Methods::new(),
            );
            let ast::BindValue::Body { ret, .. } = &specialized.value else {
                panic!("expected when expression")
            };
            let Expr::When(when) = &ret.values[0].value else {
                panic!("expected when expression")
            };
            let (pattern, body) = match &when.arms[0] {
                ast::WhenArm::Is { pattern, body, .. } => (pattern, body),
                ast::WhenArm::Cond {
                    condition: ast::Condition::Is { pattern, .. },
                    body,
                    ..
                } => (pattern, body),
                _ => panic!("expected pattern arm"),
            };
            assert_eq!(
                pattern.value.binding_names(),
                vec![Intern::from_ref("element")]
            );
            let Expr::FnCall(local) = &body.value else {
                panic!("expected pattern-local lookup")
            };
            assert_eq!(local.path.root.as_str(), "element");
            let ast::WhenArm::Else(body, _) = &when.arms[1] else {
                panic!("expected else arm")
            };
            let Expr::FnCall(outer) = &body.value else {
                panic!("expected outer constructor call")
            };
            assert_eq!(outer.path.root.as_str(), "Word");
        }
    }

    #[test]
    fn specialization_preserves_condition_pattern_scope() {
        let parsed = "outer(element, value element):\n    when value is element then\n        observed: element\n        return element\n    return element(1)\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let original = outer_bind(&parsed);
        for (form, binds) in [(0, true), (1, true), (2, false), (3, true)] {
            let mut original = original.clone();
            let ast::BindValue::Body { exprs, .. } = &mut original.value else {
                panic!("expected outer body")
            };
            let Expr::When(branch) = &mut exprs[0].value else {
                panic!("expected when expression")
            };
            let ast::WhenArm::Is {
                pattern,
                body,
                arm_span,
            } = branch.arms.remove(0)
            else {
                panic!("expected pattern arm")
            };
            let leaf = ast::Condition::Is {
                subject: branch.subject.take().unwrap(),
                pattern,
            };
            let condition = match form {
                0 => leaf,
                1 => ast::Condition::And(Box::new(leaf.clone()), Box::new(leaf)),
                2 => ast::Condition::Not(Box::new(leaf)),
                _ => ast::Condition::Or(Box::new(leaf.clone()), Box::new(leaf)),
            };
            branch.arms.push(ast::WhenArm::Cond {
                condition,
                body,
                arm_span,
            });
            let specialized = specialize(
                &original,
                Intern::from_ref("outer$specialized"),
                &Arguments {
                    values: BTreeMap::from([(
                        Intern::from_ref("element"),
                        Expr::AnonymousTag(Intern::from_ref("Word")),
                    )]),
                    receivers: BTreeMap::new(),
                },
                &Methods::new(),
            );
            let ast::BindValue::Body { exprs, ret } = &specialized.value else {
                panic!("expected outer body")
            };
            let Expr::When(branch) = &exprs[0].value else {
                panic!("expected when expression")
            };
            let ast::WhenArm::Cond { body, .. } = &branch.arms[0] else {
                panic!("condition arm")
            };
            let Expr::Block(items) = &body.value else {
                panic!("value block")
            };
            let Expr::Return(returned) = &items.last().unwrap().value else {
                panic!("explicit return")
            };
            let Expr::Bind(observed) = &items[0].value else {
                panic!("expected observation")
            };
            let ast::BindValue::Expr(value) = &observed.value else {
                panic!("expected observation value")
            };
            for expr in [&value.value, &returned.values[0].value] {
                if binds {
                    let Expr::FnCall(local) = expr else {
                        panic!("expected pattern-local lookup")
                    };
                    assert_eq!(local.path.root.as_str(), "element");
                } else {
                    assert_eq!(*expr, Expr::AnonymousTag(Intern::from_ref("Word")));
                }
            }
            let Expr::FnCall(outer) = &ret.values[0].value else {
                panic!("expected outer constructor call")
            };
            assert_eq!(outer.path.root.as_str(), "Word");
        }
    }

    #[test]
    fn specialization_preserves_for_binders_and_restores_outer_scope() {
        let parsed = "outer(element, value element):\n    for element in [element(1)]\n        observed: element\n    return element(1)\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let specialized = specialize_outer(&parsed);
        let ast::BindValue::Body { exprs, ret } = &specialized.value else {
            panic!("expected outer body")
        };
        let Expr::Loop(ast::Loop::ForIn(loop_)) = &exprs[0].value else {
            panic!("expected for loop")
        };
        assert_eq!(
            loop_.pat.value.for_loop_pattern_names(),
            Some(vec![Intern::from_ref("element")])
        );
        struct Calls(Vec<Intern<String>>);
        impl Visitor for Calls {
            fn visit_fn_call(&mut self, call: &ast::FnCall) -> ControlFlow<()> {
                self.0.push(call.path.root);
                ast::folder::walk_fn_call(self, call)
            }
        }
        let mut iter_calls = Calls(Vec::new());
        let _ = iter_calls.visit_expr(&loop_.iter.value);
        assert_eq!(iter_calls.0, vec![Intern::from_ref("Word")]);
        let Expr::Bind(use_) = &loop_.exprs[0].value else {
            panic!("expected local use")
        };
        let ast::BindValue::Expr(value) = &use_.value else {
            panic!("expected local lookup")
        };
        let Expr::FnCall(lookup) = &value.value else {
            panic!("expected loop binder lookup")
        };
        assert_eq!(lookup.path.root.as_str(), "element");
        let Expr::FnCall(after) = &ret.values[0].value else {
            panic!("expected outer constructor call")
        };
        assert_eq!(after.path.root.as_str(), "Word");
    }

    #[test]
    fn specialization_preserves_conditional_and_loop_body_scopes() {
        for block in [
            "    when True is True then\n        element: element(1)\n        return element\n",
            "    while True is True do\n        element: element(1)\n        observed: element\n",
            "    for item in 1 to 2\n        element: element(1)\n        observed: element\n",
        ] {
            let source = format!("outer(element, value element):\n{block}    return element(1)\n");
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            let specialized = specialize_outer(&parsed);
            let ast::BindValue::Body { exprs, ret } = &specialized.value else {
                panic!("expected outer body")
            };
            let Expr::FnCall(after) = &ret.values[0].value else {
                panic!("expected outer constructor call")
            };
            assert_eq!(after.path.root.as_str(), "Word");
            let (body, returned) = match &exprs[0].value {
                Expr::When(branch) => {
                    let body = match &branch.arms[0] {
                        ast::WhenArm::Cond { body, .. } | ast::WhenArm::Is { body, .. } => body,
                        _ => panic!("conditional arm"),
                    };
                    let Expr::Block(items) = &body.value else {
                        panic!("value block")
                    };
                    let Expr::Return(returned) = &items.last().unwrap().value else {
                        panic!("explicit return")
                    };
                    (items, Some(&returned.values[0].value))
                }
                Expr::Loop(ast::Loop::While(loop_)) => (&loop_.exprs, None),
                Expr::Loop(ast::Loop::ForIn(loop_)) => (&loop_.exprs, None),
                _ => panic!("expected conditional or loop"),
            };
            let Expr::Bind(local) = &body[0].value else {
                panic!("expected local declaration")
            };
            let ast::BindValue::Expr(initializer) = &local.value else {
                panic!("expected initializer")
            };
            let Expr::FnCall(call) = &initializer.value else {
                panic!("expected specialized constructor")
            };
            assert_eq!(call.path.root.as_str(), "Word");
            let lookup = returned.unwrap_or_else(|| {
                let Expr::Bind(use_) = &body[1].value else {
                    panic!("expected local use")
                };
                let ast::BindValue::Expr(value) = &use_.value else {
                    panic!("expected local lookup")
                };
                &value.value
            });
            let Expr::FnCall(lookup) = lookup else {
                panic!("expected local lookup")
            };
            assert_eq!(lookup.path.root.as_str(), "element");
        }
    }

    #[test]
    fn specialization_preserves_sequential_local_binding_scope() {
        for nested in [false, true] {
            let source = if nested {
                "outer(element, value element):\n    inner(ignored element):\n        element: element(1)\n        return element\n    return element(1)\n"
            } else {
                "outer(element, value element):\n    element: element(1)\n    return element\n"
            };
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            let specialized = specialize_outer(&parsed);
            let ast::BindValue::Body { exprs, ret } = &specialized.value else {
                panic!("expected outer body")
            };
            let scoped = if nested {
                let Expr::Bind(inner) = &exprs[0].value else {
                    panic!("expected inner binding")
                };
                let Expr::FnCall(call) = &ret.values[0].value else {
                    panic!("expected constructor call")
                };
                assert_eq!(call.path.root.as_str(), "Word");
                inner.as_ref()
            } else {
                &specialized
            };
            let ast::BindValue::Body { exprs, ret } = &scoped.value else {
                panic!("expected scoped body")
            };
            let Expr::Bind(local) = &exprs[0].value else {
                panic!("expected local binding")
            };
            let ast::BindValue::Expr(initializer) = &local.value else {
                panic!("expected initializer")
            };
            let Expr::FnCall(call) = &initializer.value else {
                panic!("expected initializer constructor call")
            };
            assert_eq!(call.path.root.as_str(), "Word");
            let Expr::FnCall(returned) = &ret.values[0].value else {
                panic!("expected local name lookup")
            };
            assert_eq!(returned.path.root.as_str(), "element");
            assert!(returned.args.is_none());
        }
    }

    #[test]
    fn specialization_preserves_nested_type_parameter_scope() {
        let parsed = "outer(element, value element):\n    inner(element, item element) element: return item\n    capture(item element) element: return item\n    return value\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let original = outer_bind(&parsed);
        let specialized = specialize_outer(&parsed);
        assert!(
            !specialized
                .params
                .as_ref()
                .unwrap()
                .contains_key(&Intern::from_ref("element"))
        );
        assert_ne!(specialized.params, original.params);

        struct Nested(&'static str, Option<Bind>);
        impl Visitor for Nested {
            fn visit_bind(&mut self, bind: &Bind) -> ControlFlow<()> {
                if bind.name.as_str() == self.0 {
                    self.1 = Some(bind.clone());
                    return ControlFlow::Break(());
                }
                ast::folder::walk_bind(self, bind)
            }
        }
        let mut before = Nested("inner", None);
        let mut after = Nested("inner", None);
        let _ = before.visit_bind(&original);
        let _ = after.visit_bind(&specialized);
        assert!(before.1.is_some());
        assert_eq!(after.1, before.1);
        let mut captured = Nested("capture", None);
        let _ = captured.visit_bind(&specialized);
        let captured = captured.1.unwrap();
        let ParameterKind::Tagged(item) =
            &captured.params.as_ref().unwrap()[&Intern::from_ref("item")].kind
        else {
            panic!("expected a tagged runtime parameter");
        };
        assert_eq!(item.value, Expr::AnonymousTag(Intern::from_ref("Word")));
        assert_eq!(captured.return_type_name, None);
        assert_eq!(
            captured.return_tag.unwrap().value,
            Expr::AnonymousTag(Intern::from_ref("Word"))
        );
    }
}
