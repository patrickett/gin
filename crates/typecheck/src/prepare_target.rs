//! Default trait materialization and entry `target` merge from flask triple.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use diagnostic::Diagnostic;
use flask::CompileTarget;
use internment::Intern;

use crate::analysis::when_declare_is_exhaustive;
use crate::analysis::{CompTimeEvaluator, ConstEnv};
use crate::ty::Ty;
use ast::declare::{Declare, DeclareValue};
use ast::expr::{Expr, Literal, Typed};
use ast::folder::Folder;
use ast::span::SpanId;
use ast::ty_state::TyState;
use ast::type_decl::TypeNameExt;
use ast::{Bind, BindValue, FileAst, WhenArm};
use ast::{ConstValue, HashFloat};

/// Fill unassigned binds whose type declares `has Default(default: …)`.
pub fn materialize_default_binds(ast: &mut FileAst) -> Vec<Diagnostic> {
    let names: Vec<_> = ast.defs.keys().copied().collect();
    let mut const_binds: ConstEnv = names.iter().map(|n| (*n, None)).collect();
    let mut diags = Vec::new();
    let mut updates: Vec<(Intern<String>, ConstValue, Expr)> = Vec::new();

    for name in names {
        let Some(bind) = ast.defs.get(&name) else {
            continue;
        };
        if !matches!(bind.value, BindValue::Unassigned) {
            continue;
        }
        let Some(type_name) = bind_return_type_name(bind) else {
            continue;
        };
        let Some(decl) = ast.tags.get(&type_name) else {
            continue;
        };
        let Some(default_expr) = provided_trait_field_expr(decl, Intern::from_ref("default"))
        else {
            continue;
        };
        let evaluator = CompTimeEvaluator::new(&const_binds, ast);
        let Some(cv) = evaluator.eval(&default_expr.value) else {
            let name_span = bind.name_span;
            diags.push(
                Diagnostic::new(
                    "compile-time-custom",
                    format!("could not evaluate Default for `{}`", name.as_str()),
                )
                .at_span(ast.span_table.get(name_span)),
            );
            continue;
        };
        const_binds.insert(name, Some(cv.clone()));
        updates.push((
            name,
            cv.clone(),
            default_expr_value_expr(&cv, bind.name_span),
        ));
    }

    for (name, cv, expr_value) in updates {
        if let Some(bind) = ast.defs.get_mut(&name) {
            bind.value = BindValue::Expr(Box::new(Typed {
                value: expr_value,
                ty: TyState::Infer,
                const_value: Some(cv),
                span_id: bind.name_span,
            }));
        }
    }

    propagate_target_fields_in_ast(ast);
    diags
}

/// Merge entry flask triple fields into the stdlib `target` bind (special case).
pub fn apply_entry_target_merge(ast: &mut FileAst, entry: &CompileTarget) -> Vec<Diagnostic> {
    let CompileTarget::Concrete(triple) = entry else {
        return Vec::new();
    };

    let target_name = Intern::from_ref("target");
    let Some(bind) = ast.defs.get_mut(&target_name) else {
        return Vec::new();
    };

    let BindValue::Expr(expr) = &mut bind.value else {
        return Vec::new();
    };

    match expr.const_value.as_mut() {
        Some(ConstValue::Record { fields }) => {
            let mut new_fields: Vec<_> = fields.iter().cloned().collect();
            for (key, value) in triple.field_overrides() {
                if let Some((_, slot)) = new_fields.iter_mut().find(|(n, _)| n.as_str() == key) {
                    *slot = ConstValue::String(value);
                } else {
                    new_fields.push((Intern::from_ref(key), ConstValue::String(value)));
                }
            }
            *fields = new_fields.into();
        }
        Some(ConstValue::Tag { name, args, .. }) if name.as_str() == "Target" => {
            let mut field_names: Vec<_> = ast
                .tags
                .get(name)
                .and_then(|decl| decl.params.as_ref())
                .map(|params| params.keys().copied().collect())
                .unwrap_or_default();
            if field_names.is_empty() {
                field_names = vec![
                    Intern::from_ref("arch"),
                    Intern::from_ref("vendor"),
                    Intern::from_ref("os"),
                ];
            }
            let mut new_args: Vec<ConstValue> = args.to_vec();
            for (key, value) in triple.field_overrides() {
                if let Some(idx) = field_names.iter().position(|field| field.as_str() == key)
                    && idx < new_args.len()
                {
                    new_args[idx] = ConstValue::String(value);
                }
            }
            *args = new_args.into();
        }
        _ => return Vec::new(),
    }

    propagate_target_fields_in_ast(ast);
    propagate_when_subjects_in_tags(ast);
    Vec::new()
}

fn propagate_when_subjects_in_tags(ast: &mut FileAst) {
    for decl in ast.tags.values_mut() {
        if let DeclareValue::When(w) = &mut decl.value
            && let Some(subject) = &mut w.subject
        {
            propagate_record_fields_typed(subject);
        }
    }
}

fn bind_return_type_name(bind: &Bind) -> Option<Intern<String>> {
    let sp = bind.return_tag.as_ref()?;
    match &sp.value {
        Expr::AnonymousTag(name) => Some(*name),
        _ => None,
    }
}

fn provided_trait_field_expr(decl: &Declare, field: Intern<String>) -> Option<Typed<Expr>> {
    decl.provided_traits.iter().find_map(|pt| {
        pt.fields
            .iter()
            .find(|(n, _)| *n == field)
            .map(|(_, e)| e.clone())
    })
}

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
    let const_binds: ConstEnv = ast.defs.keys().map(|n| (*n, None)).collect();
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
            if let Some(r) = &mut ret.value {
                materialize_type_static_access_typed(r, expansions);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
    }
}

fn materialize_type_static_access_typed(expr: &mut Typed<Expr>, expansions: &TypeStaticExpansions) {
    if let Expr::FnCall(call) = &expr.value
        && call.args.is_none()
        && call.path.value.segments.len() == 1
    {
        let type_name = call.path.value.root;
        let field = call.path.value.segments[0];
        if let Some(cv) = lookup_type_static_expansion(expansions, type_name, field) {
            *expr = Typed {
                value: default_expr_value_expr(&cv, expr.span_id),
                ty: expr.ty.clone(),
                const_value: Some(cv),
                span_id: expr.span_id,
            };
            propagate_record_fields_typed(expr);
            return;
        }
    }
    materialize_type_static_access_expr(&mut expr.value, expansions);
}

fn materialize_type_static_access_expr(expr: &mut Expr, expansions: &TypeStaticExpansions) {
    if let Expr::FnCall(call) = expr
        && call.args.is_none()
        && call.path.value.segments.len() == 1
    {
        let type_name = call.path.value.root;
        let field = call.path.value.segments[0];
        if let Some(cv) = lookup_type_static_expansion(expansions, type_name, field) {
            // No Typed wrapper available at this level — use the Expr directly.
            // The caller (materialize_type_static_access_typed) will set the span on
            // the wrapping Typed.
            *expr = default_expr_value_expr(&cv, SpanId::INVALID);
            return;
        }
    }
    match expr {
        Expr::FnCall(c) => {
            if let Some(args) = &mut c.args {
                for a in args {
                    materialize_type_static_access_typed(a, expansions);
                }
            }
        }
        Expr::Binary(b) => {
            materialize_type_static_access_typed(&mut b.lhs, expansions);
            materialize_type_static_access_typed(&mut b.rhs, expansions);
        }
        Expr::Bind(b) => {
            if let BindValue::Expr(inner) = &mut b.value {
                materialize_type_static_access_typed(inner, expansions);
            }
        }
        Expr::When(w) => {
            if let Some(s) = &mut w.subject {
                materialize_type_static_access_typed(s, expansions);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        let _ = ast::folder::walk_condition_typed_exprs_mut(
                            condition,
                            &mut |subject| {
                                materialize_type_static_access_typed(subject, expansions);
                                std::ops::ControlFlow::Continue(())
                            },
                        );
                        materialize_type_static_access_typed(body, expansions);
                    }
                    WhenArm::Is { body, .. } => {
                        materialize_type_static_access_typed(body, expansions);
                    }
                    WhenArm::Else(body, _) => {
                        materialize_type_static_access_typed(body, expansions);
                    }
                }
            }
        }
        Expr::If(i) => {
            let _ = ast::folder::walk_condition_typed_exprs_mut(&mut i.condition, &mut |subject| {
                materialize_type_static_access_typed(subject, expansions);
                std::ops::ControlFlow::Continue(())
            });
            for e in &mut i.body {
                materialize_type_static_access_typed(e, expansions);
            }
            if let Some(r) = &mut i.ret.value {
                materialize_type_static_access_typed(r, expansions);
            }
        }
        Expr::Destructure { value, .. } => {
            materialize_type_static_access_typed(value, expansions);
        }
        Expr::RecordSet { base, value, .. } => {
            materialize_type_static_access_typed(base, expansions);
            materialize_type_static_access_typed(value, expansions);
        }
        Expr::RecordGet { base, .. } => materialize_type_static_access_typed(base, expansions),
        Expr::TagCall(tc) => {
            for a in &mut tc.args {
                materialize_type_static_access_typed(a, expansions);
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                materialize_type_static_access_typed(e, expansions);
            }
        }
        _ => {}
    }
}

fn default_expr_value_expr(cv: &ConstValue, span_id: SpanId) -> Expr {
    match cv {
        ConstValue::ResultAlternative { label, .. } => Expr::AnonymousTag(*label),
        ConstValue::String(s) => Expr::Lit(Literal::String(s.clone())),
        ConstValue::Int(n) => Expr::Lit(Literal::Int(*n)),
        ConstValue::Float(HashFloat(f)) => Expr::Lit(Literal::Float(HashFloat(*f))),
        ConstValue::Tag { name, args, .. } if args.is_empty() => Expr::AnonymousTag(*name),
        ConstValue::Tag {
            name,
            qual_path,
            args,
        } => Expr::TagCall(ast::TagCall {
            name: *name,
            qual_path: qual_path.as_ref().map(|s| {
                let parts: Vec<_> = s.split('.').map(Intern::from_ref).collect();
                let root = parts[0];
                let segments = parts.get(1..).unwrap_or(&[]).to_vec();
                ast::Spanned::new(ast::ModPath::new(root, segments), span_id)
            }),
            args: args
                .iter()
                .map(|a| Typed::infer(default_expr_value_expr(a, span_id), span_id))
                .collect(),
        }),
        ConstValue::Record { fields } => {
            // Reconstruct a record literal with named fields.
            Expr::RecordLit(
                fields
                    .iter()
                    .map(|(name, v)| {
                        let value = default_expr_value_expr(v, span_id);
                        let typed = Typed::infer(value, span_id);
                        (*name, typed)
                    })
                    .collect(),
            )
        }
        ConstValue::List(items) => Expr::List(
            items
                .iter()
                .map(|i| Typed::infer(default_expr_value_expr(i, span_id), span_id))
                .collect(),
        ),
    }
}

fn propagate_target_fields_in_ast(ast: &mut FileAst) {
    for bind in ast.defs.values_mut() {
        if let BindValue::Expr(e) = &mut bind.value {
            propagate_record_fields_typed(e);
        } else if let BindValue::Body { exprs, ret } = &mut bind.value {
            for e in exprs {
                propagate_record_fields_typed(e);
            }
            if let Some(r) = &mut ret.value {
                propagate_record_fields_typed(r);
            }
        }
    }
}

/// Compile-time bind values from a prepared package AST (for cross-file `when` subjects).
pub(crate) fn const_env_from_prepared_ast(ast: &FileAst) -> ConstEnv {
    let mut const_binds: ConstEnv = ast.defs.keys().map(|n| (*n, None)).collect();
    for _ in 0..ast.defs.len().max(1) {
        let names: Vec<_> = ast.defs.keys().copied().collect();
        for name in names {
            if const_binds.get(&name).and_then(|v| v.as_ref()).is_some() {
                continue;
            }
            let Some(bind) = ast.defs.get(&name) else {
                continue;
            };
            let cv = match &bind.value {
                BindValue::Expr(e) => {
                    let evaluator = CompTimeEvaluator::new(&const_binds, ast);
                    e.const_value.clone().or_else(|| evaluator.eval(&e.value))
                }
                _ => None,
            };
            if let Some(cv) = cv {
                const_binds.insert(name, Some(cv));
            }
        }
    }
    const_binds
}

pub fn materialize_target_dependent_constants(ast: &mut FileAst, target: &CompileTarget) {
    let mut const_binds: ConstEnv = ast.defs.keys().map(|name| (*name, None)).collect();
    let target_dependent = target_dependent_bind_names(ast);
    for _ in 0..ast.defs.len().max(1) {
        for (name, bind) in &ast.defs {
            if const_binds.get(name).and_then(Option::as_ref).is_some() {
                continue;
            }
            let BindValue::Expr(expr) = &bind.value else {
                continue;
            };
            let evaluator = CompTimeEvaluator::for_target(&const_binds, ast, target);
            if let Some(value) = expr
                .const_value
                .clone()
                .or_else(|| evaluator.eval(&expr.value))
            {
                const_binds.insert(*name, Some(value));
            }
        }
    }

    for (name, bind) in &mut ast.defs {
        if !target_dependent.contains(name) {
            continue;
        }
        let Some(value) = const_binds.get(name).and_then(Clone::clone) else {
            continue;
        };
        if let BindValue::Expr(expr) = &mut bind.value {
            expr.value = default_expr_value_expr(&value, expr.span_id);
            expr.const_value = Some(value);
        }
    }
    let _ = PredicateConstantSubstituter { env: &const_binds }.visit_file_ast(ast);
    for declaration in ast.tags.values_mut() {
        if let DeclareValue::Refinement(predicate) = &mut declaration.value {
            substitute_predicate_constants(predicate, &const_binds);
        }
        if let DeclareValue::Has(members) = &mut declaration.value {
            for member in members {
                match member {
                    ast::HasMember::Property(property) => {
                        if let Some(predicate) = &mut property.refinement {
                            substitute_predicate_constants(predicate, &const_binds);
                        }
                    }
                    ast::HasMember::Function(function) => {
                        if let Some(predicate) = &mut function.refinement {
                            substitute_predicate_constants(predicate, &const_binds);
                        }
                        for predicate in function.param_refinements.values_mut() {
                            substitute_predicate_constants(predicate, &const_binds);
                        }
                    }
                }
            }
        }
        if let Some(bits) = declaration.attributes.bits.as_mut()
            && let Expr::FnCall(call) = &bits.value
            && call.args.is_none()
            && call.path.value.segments.is_empty()
            && let Some(value) = const_binds
                .get(&call.path.value.root)
                .and_then(Clone::clone)
        {
            bits.const_value = Some(value);
        }
    }
}

struct PredicateConstantSubstituter<'a> {
    env: &'a ConstEnv,
}

impl ast::folder::Folder for PredicateConstantSubstituter<'_> {
    fn visit_bind(&mut self, bind: &mut Bind) -> ControlFlow<()> {
        if let Some(predicate) = &mut bind.return_refinement {
            substitute_predicate_constants(predicate, self.env);
        }
        for predicate in bind.param_refinements.values_mut() {
            substitute_predicate_constants(predicate, self.env);
        }
        for alternative in &mut bind.anonymous_result_alternatives {
            if let Some(proposition) = &mut alternative.value.proposition {
                substitute_proposition_constants(proposition, self.env);
            }
        }
        ast::folder::walk_bind_mut(self, bind)
    }

    fn visit_condition(&mut self, condition: &mut ast::Condition) -> ControlFlow<()> {
        if let ast::Condition::Is { pattern, .. } = condition
            && let ast::Pattern::InRange { bounds, .. } = &mut pattern.value
            && let ast::InRangeBounds::LiteralToTag(min, name) = bounds
            && let Some(ConstValue::Int(max)) = self.env.get(name).and_then(Option::as_ref)
        {
            *bounds = ast::InRangeBounds::Literal(*min, *max);
        }
        ast::folder::walk_condition_mut(self, condition)
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
                names.insert(*name);
            }
        }
        if names.len() == before {
            break;
        }
    }
    names
}

fn expr_depends_on_target(expr: &Expr, dependent_names: &HashSet<Intern<String>>) -> bool {
    if matches!(expr, Expr::TargetQuery { .. })
        || matches!(
            expr,
            Expr::FnCall(call)
                if call.args.is_none()
                    && call.path.value.segments.is_empty()
                    && dependent_names.contains(&call.path.value.root)
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

/// Subject type for `when target.arch is` — the `Architecture` literal union when present.
pub fn infer_when_declare_subject_ty(
    subject: Option<&Typed<Expr>>,
    tags: &ast::TagMap,
) -> Option<Ty> {
    let subject = subject?;
    let is_target_arch = match &subject.value {
        Expr::RecordGet { field, .. } => field.as_str() == "arch",
        Expr::FnCall(call) if call.args.is_none() && call.path.root.as_str() == "target" => call
            .path
            .segments
            .first()
            .is_some_and(|s| s.as_str() == "arch"),
        _ => false,
    };
    if !is_target_arch {
        return None;
    }
    literal_union_ty_from_tag(Intern::from_ref("Architecture"), tags)
}

fn literal_union_ty_from_tag(name: Intern<String>, tags: &ast::TagMap) -> Option<Ty> {
    let decl = tags.get(&name)?;
    let DeclareValue::Union { variants } = &decl.value else {
        return None;
    };
    let mut lit_values = Vec::new();
    let mut lit_base = None;
    for variant in variants {
        let shape = variant.shape();
        if let ast::Pattern::Literal(Literal::String(s), _) = &shape.value {
            lit_values.push(ConstValue::String(s.clone()));
            if lit_base.is_none() {
                lit_base = Some(Ty::Opaque(Intern::<String>::from_ref("String")));
            }
        } else if let ast::Pattern::Literal(lit, _) = &shape.value {
            if let Some(cv) = const_value_from_literal(lit) {
                lit_values.push(cv);
            }
        } else {
            return None;
        }
    }
    if lit_values.is_empty() {
        return None;
    }
    let _ = lit_base;
    Some(Ty::union_of_literals(name, lit_values))
}

fn const_value_from_literal(lit: &Literal) -> Option<ConstValue> {
    match lit {
        Literal::String(s) => Some(ConstValue::String(s.clone())),
        Literal::Int(n) => Some(ConstValue::Int(*n)),
        Literal::Float(HashFloat(f)) => Some(ConstValue::Float(HashFloat(*f))),
        Literal::Number(n) => Some(ConstValue::Int((*n as u128).into())),
    }
}

/// `MissingElseArm` / `UnreachableElseArm` for type-level `when` before compile-time simplification.
pub fn validate_when_declare_exhaustiveness(
    ast: &FileAst,
    tag_source: &ast::TagMap,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for decl in ast.tags.values() {
        let DeclareValue::When(w) = &decl.value else {
            continue;
        };
        let subject_ty = infer_when_declare_subject_ty(w.subject.as_deref(), tag_source);
        let has_else = w.arms.iter().any(|a| matches!(a, WhenArm::Else(..)));
        let exhaustive = when_declare_is_exhaustive(subject_ty.as_ref(), &w.arms);
        if !has_else && !exhaustive {
            let span = w
                .subject
                .as_ref()
                .map(|s| ast.span_table.get(s.span_id))
                .unwrap_or_else(|| ast.span_table.get(w.body_span.into_inner()));
            diags.push(
                Diagnostic::new("type-missing-else-arm", "`when` should have an `else` arm")
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
                Diagnostic::new("type-unreachable-else-arm", "`else` arm is unreachable")
                    .at_span(ast.span_table.get(else_span.into_inner())),
            );
        }
    }
    diags
}

/// Fold `when` declare subjects (e.g. `target.arch`) using defs from the whole prepared package.
pub fn materialize_when_declare_subjects_from_package(ast: &mut FileAst, package: &FileAst) {
    let const_binds = const_env_from_prepared_ast(package);
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
                subject.const_value = Some(cv);
            }
            propagate_record_fields_typed(subject);
        }
    }
}

fn propagate_record_fields_typed(expr: &mut Typed<Expr>) {
    if let Expr::RecordGet { base, field } = &expr.value {
        let record = base.const_value.as_ref().or_else(|| {
            if let Expr::Bind(b) = &base.value
                && b.params.is_none()
                && let BindValue::Expr(inner) = &b.value
            {
                inner.const_value.as_ref()
            } else {
                None
            }
        });
        if let Some(ConstValue::Record { fields }) = record
            && let Some((_, v)) = fields.iter().find(|(n, _)| n == field)
        {
            expr.const_value = Some(v.clone());
        }
    }
    match &mut expr.value {
        Expr::FnCall(c) => {
            if let Some(args) = &mut c.args {
                for a in args {
                    propagate_record_fields_typed(a);
                }
            }
        }
        Expr::Binary(b) => {
            propagate_record_fields_typed(&mut b.lhs);
            propagate_record_fields_typed(&mut b.rhs);
        }
        Expr::Bind(b) => {
            if let BindValue::Expr(inner) = &mut b.value {
                propagate_record_fields_typed(inner);
            }
        }
        Expr::When(w) => {
            if let Some(s) = &mut w.subject {
                propagate_record_fields_typed(s);
            }
            for arm in &mut w.arms {
                match arm {
                    WhenArm::Cond {
                        condition, body, ..
                    } => {
                        let _ = ast::folder::walk_condition_typed_exprs_mut(
                            condition,
                            &mut |subject| {
                                propagate_record_fields_typed(subject);
                                std::ops::ControlFlow::Continue(())
                            },
                        );
                        propagate_record_fields_typed(body);
                    }
                    WhenArm::Is { body, .. } => propagate_record_fields_typed(body),
                    WhenArm::Else(body, _) => propagate_record_fields_typed(body),
                }
            }
        }
        Expr::If(i) => {
            let _ = ast::folder::walk_condition_typed_exprs_mut(&mut i.condition, &mut |subject| {
                propagate_record_fields_typed(subject);
                std::ops::ControlFlow::Continue(())
            });
            for e in &mut i.body {
                propagate_record_fields_typed(e);
            }
            if let Some(r) = &mut i.ret.value {
                propagate_record_fields_typed(r);
            }
        }
        Expr::Destructure { value, .. } => {
            propagate_record_fields_typed(value);
        }
        Expr::RecordSet { base, value, .. } => {
            propagate_record_fields_typed(base);
            propagate_record_fields_typed(value);
        }
        Expr::RecordGet { base, .. } => propagate_record_fields_typed(base),
        Expr::TagCall(tc) => {
            for a in &mut tc.args {
                propagate_record_fields_typed(a);
            }
        }
        Expr::TupleLit(elems) | Expr::List(elems) => {
            for e in elems {
                propagate_record_fields_typed(e);
            }
        }
        _ => {}
    }
}
