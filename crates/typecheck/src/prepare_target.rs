//! Compile-time preparation for explicit target queries and type declarations.

use diagnostic::code::PatternCode;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use diagnostic::Diagnostic;
use flask::CompileTarget;
use internment::Intern;

use crate::analysis::when_declare_is_exhaustive;
use crate::analysis::{CompTimeEvaluator, ConstEnv};
use ast::ConstValue;
use ast::declare::DeclareValue;
use ast::expr::{Expr, Literal, Typed};
use ast::folder::Folder;
use ast::span::SpanId;
use ast::type_decl::TypeNameExt;
use ast::{Bind, BindValue, FileAst, WhenArm};

#[cfg(test)]
#[path = "prepare_target_tests.rs"]
mod tests;

type TypeStaticKey = (Intern<String>, Intern<String>);
type TypeStaticExpansions = HashMap<TypeStaticKey, ConstValue>;

fn build_type_static_expansions(ast: &FileAst, const_binds: &ConstEnv) -> TypeStaticExpansions {
    let mut out = TypeStaticExpansions::new();
    for (type_name, decl) in &ast.tags {
        if !type_name.as_str().is_capitalized_type_name() {
            continue;
        }
        for pt in &decl.provided_traits {
            for (field, expr) in &pt.fields {
                let evaluator = CompTimeEvaluator::new(const_binds, ast);
                if let Some(cv) = evaluator.eval(&expr.value) {
                    out.insert((*type_name, *field), cv);
                }
            }
        }
    }
    out
}

fn lookup_type_static_expansion(
    expansions: &TypeStaticExpansions,
    type_name: Intern<String>,
    field: Intern<String>,
) -> Option<ConstValue> {
    if !type_name.as_str().is_capitalized_type_name() {
        return None;
    }
    expansions.get(&(type_name, field)).cloned()
}

/// Evaluate a static member supplied by one of the type's nominal traits.
pub(crate) fn eval_type_static_member(
    type_name: Intern<String>,
    field: Intern<String>,
    ast: &FileAst,
    const_binds: &ConstEnv,
) -> Option<ConstValue> {
    let expansions = build_type_static_expansions(ast, const_binds);
    lookup_type_static_expansion(&expansions, type_name, field)
}

/// Expand `Target.default`-style paths to concrete compile-time expressions.
pub fn materialize_type_static_access(ast: &mut FileAst) {
    let const_binds: ConstEnv = ast.defs.keys().map(|n| (n.0, None)).collect();
    let expansions = build_type_static_expansions(ast, &const_binds);
    let bind_names: Vec<_> = ast.defs.keys().copied().collect();
    for name in bind_names {
        if let Some(bind) = ast.defs.get_mut(&name) {
            materialize_type_static_access_in_bind(bind, &expansions);
        }
    }
    for (expr, _) in &mut ast.exprs {
        materialize_type_static_access_expr(expr, &expansions);
    }
    let tag_names: Vec<_> = ast.tags.keys().copied().collect();
    for tag_name in tag_names {
        let Some(decl) = ast.tags.get_mut(&tag_name) else {
            continue;
        };
        if let DeclareValue::When(w) = &mut decl.value {
            if let Some(subject) = &mut w.subject {
                materialize_type_static_access_typed(subject, &expansions);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        let _ = ast::folder::walk_condition_typed_exprs_mut(
                            condition,
                            &mut |subject| {
                                materialize_type_static_access_typed(subject, &expansions);
                                std::ops::ControlFlow::Continue(())
                            },
                        );
                        materialize_type_static_access_typed(body, &expansions);
                    }
                    WhenArm::Is { body, .. } => {
                        materialize_type_static_access_typed(body, &expansions);
                    }
                    WhenArm::Else(body, _) => {
                        materialize_type_static_access_typed(body, &expansions);
                    }
                }
            }
        }
    }
}

fn materialize_type_static_access_in_bind(bind: &mut Bind, expansions: &TypeStaticExpansions) {
    match &mut bind.value {
        BindValue::Expr(e) => materialize_type_static_access_typed(e, expansions),
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                materialize_type_static_access_typed(e, expansions);
            }
            for r in &mut ret.values {
                materialize_type_static_access_typed(r, expansions);
            }
        }
        BindValue::Unassigned => {}
    }
}

fn type_static_expansion_of(expr: &Expr, expansions: &TypeStaticExpansions) -> Option<ConstValue> {
    let Expr::FnCall(call) = expr else {
        return None;
    };
    if call.args.is_some() || call.path.value.segments.len() != 1 {
        return None;
    }
    lookup_type_static_expansion(
        expansions,
        call.path.value.root,
        call.path.value.segments[0],
    )
}

fn materialize_type_static_access_typed(expr: &mut Typed<Expr>, expansions: &TypeStaticExpansions) {
    if let Some(cv) = type_static_expansion_of(&expr.value, expansions) {
        *expr = Typed {
            value: crate::const_value::const_value_to_inferred_expr(&cv, expr.span_id),
            ty: expr.ty.clone(),
            const_value: Some(Box::new(cv)),
            span_id: expr.span_id,
        };
        propagate_record_fields_typed(expr);
        return;
    }
    materialize_type_static_access_expr(&mut expr.value, expansions);
}

fn materialize_type_static_access_expr(expr: &mut Expr, expansions: &TypeStaticExpansions) {
    if let Some(cv) = type_static_expansion_of(expr, expansions) {
        // No Typed wrapper available at this level — use the Expr directly.
        // The caller (materialize_type_static_access_typed) will set the span on
        // the wrapping Typed.
        *expr = crate::const_value::const_value_to_inferred_expr(&cv, SpanId::INVALID);
        return;
    }
    let _ = ast::folder::walk_typed_expr_children_mut(expr, &mut |child| {
        materialize_type_static_access_typed(child, expansions);
        ControlFlow::Continue(())
    });
}

/// Compile-time bind values from a prepared package AST (for cross-file `when` subjects).
pub(crate) fn const_env_from_prepared_ast(
    ast: &FileAst,
    target: Option<&CompileTarget>,
) -> ConstEnv {
    let mut const_binds: ConstEnv = ast.defs.keys().map(|n| (n.0, None)).collect();
    for _ in 0..ast.defs.len().max(1) {
        let mut changed = false;
        for (name, bind) in &ast.defs {
            if bind
                .params
                .as_ref()
                .is_some_and(|params| !params.is_empty())
            {
                continue;
            }
            if const_binds.get(&name.0).and_then(Option::as_ref).is_some() {
                continue;
            }
            let cv = match &bind.value {
                BindValue::Expr(e) => {
                    let evaluator = match target {
                        Some(target) => CompTimeEvaluator::for_target(&const_binds, ast, target),
                        None => CompTimeEvaluator::new(&const_binds, ast),
                    };
                    e.const_value
                        .as_deref()
                        .cloned()
                        .or_else(|| evaluator.eval(&e.value))
                }
                _ => None,
            };
            if let Some(cv) = cv {
                const_binds.insert(name.0, Some(cv));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    const_binds
}

pub fn materialize_target_dependent_constants(ast: &mut FileAst, target: &CompileTarget) {
    materialize_target_when_subjects(ast, target);
    let const_binds = const_env_from_prepared_ast(ast, Some(target));
    let target_dependent = target_dependent_bind_names(ast);

    for (name, bind) in &mut ast.defs {
        if !target_dependent.contains(&name.0) {
            continue;
        }
        let Some(value) = const_binds.get(&name.0).and_then(Clone::clone) else {
            continue;
        };
        if let BindValue::Expr(expr) = &mut bind.value {
            if matches!(
                value,
                ConstValue::Int(_) | ConstValue::Float(_) | ConstValue::Text(_)
            ) {
                expr.value = crate::const_value::const_value_to_inferred_expr(&value, expr.span_id);
            }
            expr.const_value = Some(Box::new(value));
        }
    }
    materialize_constant_predicates(ast, &const_binds);
}

pub(crate) fn materialize_constant_predicates(ast: &mut FileAst, const_binds: &ConstEnv) {
    let receiver_parameters: HashMap<_, HashSet<_>> = ast
        .tags
        .iter()
        .map(|(name, declaration)| {
            (
                *name,
                declaration
                    .params
                    .iter()
                    .flat_map(|parameters| parameters.keys().copied())
                    .collect(),
            )
        })
        .collect();
    let _ = PredicateConstantSubstituter {
        env: const_binds,
        receiver_parameters: &receiver_parameters,
    }
    .visit_file_ast(ast);
    for declaration in ast.tags.values_mut() {
        let fields = match &declaration.value {
            DeclareValue::Has(members) => members
                .iter()
                .filter_map(|member| match member {
                    ast::HasMember::Property(property) if property.qualifier.is_none() => {
                        Some(property.name)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let constants = scoped_const_env(
            const_binds,
            declaration
                .params
                .iter()
                .flat_map(|parameters| parameters.keys().copied())
                .chain(fields),
        );
        if let DeclareValue::Refinement(predicate) = &mut declaration.value {
            substitute_predicate_constants(predicate, &constants);
        }
        if let DeclareValue::Has(members) = &mut declaration.value {
            for member in members {
                match member {
                    ast::HasMember::Property(property) => {
                        if let Some(surface) = &mut property.ty {
                            let _ = TypeConstantSubstituter {
                                values: &constants,
                                parameters: declaration.params.as_ref(),
                            }
                            .visit_expr(&mut surface.value);
                        }
                        if let Some(predicate) = &mut property.refinement {
                            substitute_predicate_constants(predicate, &constants);
                        }
                    }
                    ast::HasMember::Function(function) => {
                        let constants =
                            scoped_const_env(&constants, function.params.keys().copied());
                        if let Some(predicate) = &mut function.refinement {
                            substitute_predicate_constants(predicate, &constants);
                        }
                        for predicate in function.param_refinements.values_mut() {
                            substitute_predicate_constants(predicate, &constants);
                        }
                        for proof in &mut function.payload_postconditions {
                            let scoped = scoped_const_env(&constants, [proof.name]);
                            substitute_proposition_constants(&mut proof.proposition, &scoped);
                        }
                        for alternative in &mut function.anonymous_result_alternatives {
                            if let Some(proposition) = &mut alternative.value.proposition {
                                substitute_proposition_constants(proposition, &constants);
                            }
                        }
                    }
                }
            }
        }
        if let Some(bits) = declaration.attributes.bits.as_mut()
            && let Expr::FnCall(call) = &bits.value
            && call.args.is_none()
            && let Some(value) = constants
                .get(&Intern::new(call.path.value.to_qualified_string()))
                .and_then(Clone::clone)
        {
            bits.const_value = Some(Box::new(value));
        }
    }
}

fn materialize_target_when_subjects(ast: &mut FileAst, target: &CompileTarget) {
    if matches!(target, CompileTarget::Library) {
        return;
    }
    let constants = ConstEnv::default();
    let expressions = FileAst::default();
    let mut folder = TargetWhenSubjects {
        evaluator: CompTimeEvaluator::for_target(&constants, &expressions, target),
    };
    let _ = folder.visit_file_ast(ast);
    for declaration in ast.tags.values_mut() {
        if let DeclareValue::When(when) = &mut declaration.value {
            let _ = folder.visit_when_expr(when);
        }
    }
}

struct TargetWhenSubjects<'a> {
    evaluator: CompTimeEvaluator<'a>,
}

impl Folder for TargetWhenSubjects<'_> {
    fn visit_when_expr(&mut self, when: &mut ast::WhenExpr) -> ControlFlow<()> {
        if let Some(subject) = &mut when.subject
            && expr_depends_on_target(&subject.value, &HashSet::new())
            && let Some(value) = self.evaluator.eval(&subject.value)
        {
            subject.const_value = Some(Box::new(value));
        }
        ast::folder::walk_when_mut(self, when)
    }
}

fn scoped_const_env(
    env: &ConstEnv,
    names: impl IntoIterator<Item = Intern<String>>,
) -> Cow<'_, ConstEnv> {
    let names: HashSet<_> = names.into_iter().collect();
    if names.is_empty() {
        return Cow::Borrowed(env);
    }
    let shadowed = |name: &Intern<String>| {
        let name = name.as_str();
        let root = name.split_once('.').map_or(name, |(root, _)| root);
        names.contains(&Intern::from_ref(root))
    };
    if !env.keys().any(shadowed) {
        return Cow::Borrowed(env);
    }
    let mut scoped = env.clone();
    scoped.retain(|name, _| !shadowed(name));
    Cow::Owned(scoped)
}

struct TypeConstantSubstituter<'a> {
    values: &'a ConstEnv,
    parameters: Option<&'a ast::Parameters>,
}

impl ast::folder::Folder for TypeConstantSubstituter<'_> {
    fn visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<()> {
        if let Expr::FnCall(call) = expr
            && call.args.is_none()
            && !self
                .parameters
                .is_some_and(|params| params.contains_key(&call.path.root))
            && let Some(Some(ConstValue::Int(value))) = self
                .values
                .get(&Intern::new(call.path.value.to_qualified_string()))
        {
            *expr = Expr::Lit(Literal::Int(*value));
            return ControlFlow::Continue(());
        }
        ast::folder::walk_expr_mut(self, expr)
    }
}

struct PredicateConstantSubstituter<'a> {
    env: &'a ConstEnv,
    receiver_parameters: &'a HashMap<Intern<String>, HashSet<Intern<String>>>,
}

impl ast::folder::Folder for PredicateConstantSubstituter<'_> {
    fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        let receiver_parameters = bind
            .receiver_type_surface()
            .and_then(|surface| {
                let Expr::TagCall(receiver) = &surface.value else {
                    return None;
                };
                let name = receiver.qualifier().map_or(receiver.name, |qualifier| {
                    Intern::new(format!("{qualifier}.{}", receiver.name))
                });
                let parameters = self.receiver_parameters.get(&name)?;
                Some(
                    receiver
                        .args
                        .iter()
                        .filter_map(|argument| match &argument.value {
                            Expr::AnonymousTag(name) if parameters.contains(name) => Some(*name),
                            _ => None,
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default();
        let env = scoped_const_env(
            self.env,
            bind.params
                .iter()
                .flat_map(|parameters| parameters.keys().copied())
                .chain(receiver_parameters),
        );
        let mut types = TypeConstantSubstituter {
            values: &env,
            parameters: None,
        };
        if let Some(surface) = &mut bind.return_tag {
            let _ = types.visit_expr(&mut surface.value);
        }
        for result in &mut bind.named_results {
            let _ = types.visit_expr(&mut result.ty.value);
        }
        if let Some(parameters) = &mut bind.params {
            for parameter in parameters.values_mut() {
                match &mut parameter.kind {
                    ast::ParameterKind::Tagged(surface)
                    | ast::ParameterKind::ValueParam { ty: surface }
                    | ast::ParameterKind::Inferred { ty: surface } => {
                        let _ = types.visit_expr(&mut surface.value);
                    }
                    _ => {}
                }
            }
        }
        if let Some(predicate) = &mut bind.return_refinement {
            substitute_predicate_constants(predicate, &env);
        }
        for predicate in bind.param_refinements.values_mut() {
            substitute_predicate_constants(predicate, &env);
        }
        for proof in &mut bind.payload_postconditions {
            let scoped = scoped_const_env(&env, [proof.name]);
            substitute_proposition_constants(&mut proof.proposition, &scoped);
        }
        for alternative in &mut bind.anonymous_result_alternatives {
            if let Some(proposition) = &mut alternative.value.proposition {
                substitute_proposition_constants(proposition, &env);
            }
        }
        ast::folder::walk_bind_mut(
            &mut PredicateConstantSubstituter {
                env: &env,
                receiver_parameters: self.receiver_parameters,
            },
            bind,
        )
    }

    fn visit_condition(&mut self, condition: &mut ast::Condition) -> ControlFlow<()> {
        if let ast::Condition::Is { pattern, .. } = condition {
            substitute_pattern_bounds(&mut pattern.value, self.env);
        }
        ast::folder::walk_condition_mut(self, condition)
    }

    fn visit_when_arm(&mut self, arm: &mut ast::WhenArm) -> ControlFlow<()> {
        if let ast::WhenArm::Is { pattern, .. } = arm {
            substitute_pattern_bounds(&mut pattern.value, self.env);
        }
        ast::folder::walk_when_arm_mut(self, arm)
    }
}

fn substitute_pattern_bounds(pattern: &mut ast::Pattern, env: &ConstEnv) {
    if let ast::Pattern::InRange { bounds, .. } = pattern
        && let ast::InRangeBounds::LiteralToTag(min, name) = bounds
        && let Some(ConstValue::Int(max)) = env.get(name).and_then(Option::as_ref)
    {
        *bounds = ast::InRangeBounds::Literal(*min, *max);
    }
}

pub(crate) fn target_dependent_bind_names(ast: &FileAst) -> HashSet<Intern<String>> {
    let mut names = HashSet::new();
    for _ in 0..ast.defs.len().max(1) {
        let before = names.len();
        for (name, bind) in &ast.defs {
            if let BindValue::Expr(expr) = &bind.value
                && expr_depends_on_target(&expr.value, &names)
            {
                names.insert(name.0);
            }
        }
        if names.len() == before {
            break;
        }
    }
    names
}

fn expr_depends_on_target(expr: &Expr, dependent_names: &HashSet<Intern<String>>) -> bool {
    if matches!(expr, Expr::Target | Expr::TargetQuery { .. })
        || matches!(
            expr,
            Expr::FnCall(call)
                if call.args.is_none()
                    && dependent_names.contains(&Intern::new(call.path.value.to_qualified_string()))
        )
    {
        return true;
    }
    let mut found = false;
    let _ = ast::folder::walk_expr_children(expr, &mut |_, child| {
        if expr_depends_on_target(child, dependent_names) {
            found = true;
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    });
    found
}

fn substitute_predicate_constants(predicate: &mut ast::ty::PredicateExpr, env: &ConstEnv) {
    match predicate {
        ast::ty::PredicateExpr::Lt(expr)
        | ast::ty::PredicateExpr::Gt(expr)
        | ast::ty::PredicateExpr::Le(expr)
        | ast::ty::PredicateExpr::Ge(expr)
        | ast::ty::PredicateExpr::Eq(expr)
        | ast::ty::PredicateExpr::Ne(expr) => substitute_normal_constants(expr, env),
        ast::ty::PredicateExpr::And(predicates) => {
            for predicate in predicates {
                substitute_predicate_constants(predicate, env);
            }
        }
        ast::ty::PredicateExpr::Proposition(proposition) => {
            substitute_proposition_constants(proposition, env);
        }
    }
}

fn substitute_normal_constants(expr: &mut ast::NormalExpr, env: &ConstEnv) {
    match expr {
        ast::NormalExpr::Var(name) => {
            if let Some(ConstValue::Int(value)) = env.get(name).and_then(Clone::clone) {
                *expr = ast::NormalExpr::from(value);
            }
        }
        ast::NormalExpr::Add(left, right)
        | ast::NormalExpr::Sub(left, right)
        | ast::NormalExpr::Mul(left, right) => {
            substitute_normal_constants(left, env);
            substitute_normal_constants(right, env);
        }
        ast::NormalExpr::Value(_)
        | ast::NormalExpr::Inferred(_)
        | ast::NormalExpr::TargetQuery { .. } => {}
    }
}

fn substitute_proposition_constants(proposition: &mut ast::ProofProposition, env: &ConstEnv) {
    match proposition {
        ast::ProofProposition::Compare { left, right, .. } => {
            substitute_proof_term_constants(left, env);
            substitute_proof_term_constants(right, env);
        }
        ast::ProofProposition::InRange { value, start, end } => {
            substitute_proof_term_constants(value, env);
            substitute_proof_term_constants(start, env);
            substitute_proof_term_constants(end, env);
        }
        ast::ProofProposition::Not(inner) => substitute_proposition_constants(inner, env),
        ast::ProofProposition::And(left, right) | ast::ProofProposition::Or(left, right) => {
            substitute_proposition_constants(left, env);
            substitute_proposition_constants(right, env);
        }
    }
}

fn substitute_proof_term_constants(term: &mut ast::ProofTerm, env: &ConstEnv) {
    match term {
        ast::ProofTerm::Name(name) => {
            if let Some(ConstValue::Int(value)) = env.get(name).and_then(Clone::clone) {
                *term = ast::ProofTerm::Value(value);
            }
        }
        ast::ProofTerm::Add(left, right)
        | ast::ProofTerm::Sub(left, right)
        | ast::ProofTerm::Mul(left, right)
        | ast::ProofTerm::Remainder(left, right) => {
            substitute_proof_term_constants(left, env);
            substitute_proof_term_constants(right, env);
        }
        ast::ProofTerm::PowerOfTwo(inner) => substitute_proof_term_constants(inner, env),
        ast::ProofTerm::Value(_) | ast::ProofTerm::TargetQuery { .. } => {}
    }
}

/// `MissingElseArm` / `UnreachableElseArm` for type-level `when` before compile-time simplification.
pub fn validate_when_declare_exhaustiveness(ast: &FileAst) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for decl in ast.tags.values() {
        let DeclareValue::When(w) = &decl.value else {
            continue;
        };
        let subject_ty = w
            .subject
            .as_deref()
            .and_then(|subject| subject.ty.current_ty());
        let has_else = w.arms.iter().any(|a| matches!(a, WhenArm::Else(..)));
        let exhaustive = when_declare_is_exhaustive(subject_ty, &w.arms);
        if !has_else && !exhaustive {
            let span = w
                .subject
                .as_ref()
                .map(|s| ast.span_table.get(s.span_id))
                .unwrap_or_else(|| ast.span_table.get(w.body_span.into_inner()));
            diags.push(
                // type-missing-else-arm
                Diagnostic::new(PatternCode::ElseMissing, "`when` should have an `else` arm")
                    .at_span(span),
            );
        } else if has_else && exhaustive {
            let Some(else_span) = w.arms.iter().find_map(|a| match a {
                WhenArm::Else(_, span) => Some(*span),
                _ => None,
            }) else {
                continue;
            };
            diags.push(
                // type-unreachable-else-arm
                Diagnostic::new(PatternCode::ElseUnreachable, "`else` arm is unreachable")
                    .at_span(ast.span_table.get(else_span.into_inner())),
            );
        }
    }
    diags
}

/// Fold `when` declare subjects (e.g. `target.arch`) using defs from the whole prepared package.
pub fn materialize_when_declare_subjects_from_package(ast: &mut FileAst, package: &FileAst) {
    let const_binds = const_env_from_prepared_ast(package, None);
    for decl in ast.tags.values_mut() {
        if let DeclareValue::When(w) = &mut decl.value
            && let Some(subject) = &mut w.subject
        {
            if subject.const_value.is_none()
                && let Some(cv) = {
                    let evaluator = CompTimeEvaluator::new(&const_binds, package);
                    evaluator.eval(&subject.value)
                }
            {
                subject.const_value = Some(Box::new(cv));
            }
            propagate_record_fields_typed(subject);
        }
    }
}

fn propagate_record_fields_typed(expr: &mut Typed<Expr>) {
    let _ = ast::folder::walk_typed_expr_children_mut(&mut expr.value, &mut |child| {
        propagate_record_fields_typed(child);
        ControlFlow::Continue(())
    });
    if let Expr::RecordGet { base, field } = &expr.value {
        let record = base.const_value.as_deref().or_else(|| {
            if let Expr::Bind(b) = &base.value
                && b.params.is_none()
                && let BindValue::Expr(inner) = &b.value
            {
                inner.const_value.as_deref()
            } else {
                None
            }
        });
        if let Some(ConstValue::Record { fields }) = record
            && let Some((_, v)) = fields.iter().find(|(n, _)| n == field)
        {
            expr.const_value = Some(Box::new(v.clone()));
        }
    }
}
