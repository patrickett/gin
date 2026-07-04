//! Stage 2: Resolve — Lower parse expressions to the typed arena, attach type flaws.
//!
//! This stage walks the parse-tree expressions, infers/resolves their types,
//! converts them to [`TypedExprKind`], pushes them into the expression arena,
//! and attaches type-check flaws.

use internment::Intern;
use std::collections::{HashMap, HashSet};

use crate::analysis::when_is_exhaustive;
use crate::analysis::{eval_compile_time_expr_public, pattern_matches_public};
use crate::prepare_target::const_binds_from_prepared_ast;
use ast::HashFloat;
use ast::prelude::*;
use ast_format::type_expr::TypeExprFormatExt;

use crate::ty::Ty;
use crate::typed::{
    BindBody, DefId, ExprId, TagId, TypedExprKind, TypedFileAst, TypedWhenArm, VariantMap,
};
use ast::ConstValue;
use diagnostic::Diagnostic;

use super::TransformCtx;
use super::bounds_check::check_fn_call_bounds;
use super::lower_kind::lower_expr_kind;
use super::lower_tag::is_record_tag_call;
use super::lower_ty::{annotate_literal_union_literal, bind_explicit_ty, resolve_expr_type};
use super::lower_util::closest_name;

/// Map from local variable name to its resolved type, built incrementally
/// during expression lowering. This lets `resolve_expr_type` look up the
/// type of a variable that was bound earlier in the same function body.
pub(crate) type LocalVarTypes = HashMap<Intern<String>, Ty>;

/// Local types plus names introduced with `:=` (immutable).
#[derive(Clone, Default)]
pub(crate) struct LocalEnv {
    pub(crate) types: LocalVarTypes,
    /// Names declared in the current scope (for reassignment detection).
    pub(crate) locals: HashSet<Intern<String>>,
    /// Names introduced with `:=` (immutable) for the `ReassignConstant` check.
    pub(crate) constants: HashSet<Intern<String>>,
    /// Compile-time-known local values available while lowering this scope.
    pub(crate) const_values: HashMap<Intern<String>, ast::ConstValue>,
}

/// Lexical context while lowering expressions.
pub(crate) struct ExprLowerScope<'a> {
    pub(crate) tag_types: &'a HashMap<Intern<String>, Ty>,
    pub(crate) variant_map: &'a VariantMap,
    pub(crate) receiver_type: Option<&'a Ty>,
    /// When set (e.g. explicit function return type), bare tags like `True` resolve as that union's variants.
    pub(crate) expected_ty: Option<&'a Ty>,
    pub(crate) locals: &'a HashSet<Intern<String>>,
    /// Current definition being lowered (for SelfRef resolution).
    pub(crate) current_def_id: Option<DefId>,
}

impl<'a> ExprLowerScope<'a> {
    fn new(
        tag_types: &'a HashMap<Intern<String>, Ty>,
        variant_map: &'a VariantMap,
        receiver_type: Option<&'a Ty>,
        expected_ty: Option<&'a Ty>,
        locals: &'a HashSet<Intern<String>>,
        current_def_id: Option<DefId>,
    ) -> Self {
        Self {
            tag_types,
            variant_map,
            receiver_type,
            expected_ty,
            locals,
            current_def_id,
        }
    }

    /// Call args, binary operands, and nested `when` subjects do not inherit the outer expected return type.
    pub(crate) fn child(&self) -> Self {
        Self {
            tag_types: self.tag_types,
            variant_map: self.variant_map,
            receiver_type: self.receiver_type,
            expected_ty: None,
            locals: self.locals,
            current_def_id: self.current_def_id,
        }
    }
}

/// Walks all bind bodies and top-level expressions in the `FileAst`,
/// converts parse-tree `Expr` nodes to `TypedExprKind`, pushes them into
/// the expression arena, and attaches type-check flaws.
pub fn stage_lower(typed: &mut TypedFileAst, file_ast: &FileAst, ctx: &TransformCtx) {
    // Own tag types plus cross-file.
    let mut tag_types: HashMap<Intern<String>, Ty> = ctx
        .cross_file_tag_types
        .iter()
        .map(|(tid, ty)| (tid.0, ty.clone()))
        .collect();
    for (tid, ty) in &typed.tag_types {
        tag_types.insert(tid.0, ty.clone());
    }

    // Collect variant_map reference before any mutable borrows (own + cross-file).
    let mut variant_map: VariantMap = typed.variant_map.clone();
    for (variant_name, entries) in &ctx.cross_file_variant_map {
        variant_map
            .entry(*variant_name)
            .or_default()
            .extend(entries.iter().cloned());
    }

    // First pass: lower all expressions, collecting body assignments.
    struct DefBodyAssign {
        def_id: DefId,
        body: BindBody,
    }

    let mut def_ids: Vec<DefId> = typed.defs.keys().copied().collect();
    def_ids.sort_by_key(|def_id| {
        let is_value_expr = file_ast
            .defs
            .get(&def_id.0)
            .is_some_and(|bind| matches!(bind.value, BindValue::Expr(_)) && bind.params.is_none());
        (!is_value_expr, def_id.0.as_str().to_string())
    });

    // Pre-collect receiver types and param names.
    let receiver_types: HashMap<DefId, Option<Ty>> = def_ids
        .iter()
        .map(|def_id| {
            let recv = typed.defs.get(def_id).and_then(|b| b.receiver_type.clone());
            (*def_id, recv)
        })
        .collect();

    let param_sets: HashMap<DefId, HashSet<Intern<String>>> = def_ids
        .iter()
        .map(|def_id| {
            let mut params: HashSet<Intern<String>> = typed
                .defs
                .get(def_id)
                .map(|b| b.params.iter().map(|(n, _)| *n).collect())
                .unwrap_or_default();
            if let Some(parsed_params) = file_ast
                .defs
                .get(&def_id.0)
                .and_then(|bind| bind.params.as_ref())
            {
                params.extend(parsed_params.keys().copied());
            }
            (*def_id, params)
        })
        .collect();

    let mut assignments: Vec<DefBodyAssign> = Vec::new();

    for def_id in &def_ids {
        let Some(bind) = file_ast.defs.get(&def_id.0) else {
            continue;
        };

        let receiver_type = receiver_types.get(def_id).and_then(|r| r.as_ref());
        let empty_locals = HashSet::new();
        let locals: &HashSet<Intern<String>> = param_sets.get(def_id).unwrap_or(&empty_locals);

        let mut env = LocalEnv::default();
        if let Some(typed_bind) = typed.defs.get(def_id) {
            for (name, ty) in &typed_bind.params {
                env.types.insert(*name, ty.clone());
            }
        }

        let return_ty = typed.defs.get(def_id).map(|b| b.return_type.clone());
        let scope = ExprLowerScope::new(
            &tag_types,
            &variant_map,
            receiver_type,
            return_ty.as_ref(),
            locals,
            Some(*def_id),
        );

        let body = match &bind.value {
            BindValue::Expr(typed_expr) => {
                let id = lower_typed_expr(typed, typed_expr.as_ref(), &scope, &mut env);
                if let Some(explicit) = bind_explicit_ty(bind, &tag_types) {
                    typed.exprs.ty[id.as_usize()] = explicit.clone();
                    if let Some(values) = explicit.union_literal_values() {
                        annotate_literal_union_literal(typed, id, &explicit, values);
                    } else {
                        typed.exprs.const_value[id.as_usize()] = None;
                    }
                }
                BindBody::Expr(id)
            }
            BindValue::Body { exprs, ret } => {
                let mut lowered_exprs: Vec<ExprId> = Vec::new();
                for expr in exprs {
                    lowered_exprs.push(lower_typed_expr(typed, expr, &scope.child(), &mut env));
                }
                let ret_id = ret
                    .value
                    .as_ref()
                    .map(|ret_expr| lower_typed_expr(typed, ret_expr, &scope, &mut env));
                BindBody::Body {
                    exprs: lowered_exprs,
                    ret: ret_id,
                }
            }
            BindValue::Extern | BindValue::Unassigned => BindBody::Extern,
        };
        if let Some(typed_bind) = typed.defs.get_mut(def_id) {
            typed_bind.body = body.clone();
        }
        assignments.push(DefBodyAssign {
            def_id: *def_id,
            body,
        });
    }

    // Lower top-level expressions.
    let empty_locals = HashSet::new();
    let mut env = LocalEnv::default();
    let scope = ExprLowerScope::new(&tag_types, &variant_map, None, None, &empty_locals, None);
    for (expr, span_id) in &file_ast.exprs {
        let wrapped = Typed::infer(expr.clone(), *span_id);
        let expr_id = lower_typed_expr(typed, &wrapped, &scope, &mut env);
        typed.root_exprs.push(expr_id);
    }

    // Check for redundant `self` parameter type annotations on methods.
    for def_id in &def_ids {
        let Some(bind) = file_ast.defs.get(&def_id.0) else {
            continue;
        };
        if !bind.is_method() {
            continue;
        }
        if let Some(params) = &bind.params
            && params.iter().any(|(name, kind)| {
                name.as_str() == "self" && matches!(kind, ParameterKind::Tagged(_))
            })
            && let Some(typed_bind) = typed.defs.get_mut(def_id)
        {
            typed_bind.flaws.push(Diagnostic::new(
                "type-self-param-typed",
                "self parameter should not have a type annotation",
            ));
        }
    }

    // Second pass: assign bodies to defs.
    for assign in assignments {
        if let Some(typed_bind) = typed.defs.get_mut(&assign.def_id) {
            typed_bind.body = assign.body;
        }
    }
}

pub(crate) fn lower_typed_expr(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
) -> ExprId {
    let resolved_ty = resolve_expr_type(
        expr,
        scope.tag_types,
        scope.variant_map,
        scope.receiver_type,
        scope.expected_ty,
        &env.types,
        &typed.fn_return_types,
    );
    // Infer const_value from literals and known value refs at parse time (before const-folding runs).
    let const_val = expr.const_value.clone().or_else(|| match &expr.value {
        Expr::FnCall(call) if call.args.is_none() => {
            if call.path.value.segments.is_empty()
                && let Some(cv) = env.const_values.get(&call.path.value.root)
            {
                Some(cv.clone())
            } else if call.path.value.segments.is_empty()
                && (env.types.contains_key(&call.path.value.root)
                    || scope.locals.contains(&call.path.value.root))
            {
                None
            } else {
                let target =
                    super::lower_ty::resolve_fn_call_target(&call.path.value, scope.tag_types);
                const_for_def_id(target, typed).or_else(|| {
                    scope.variant_map.get(&target.0).and_then(|entries| {
                        if entries.len() == 1 {
                            Some(ast::ConstValue::Tag {
                                name: target.0,
                                qual_path: None,
                                args: vec![].into(),
                            })
                        } else {
                            None
                        }
                    })
                })
            }
        }
        Expr::AnonymousTag(name) => scope.variant_map.get(name).and_then(|entries| {
            if entries.len() == 1 {
                Some(ast::ConstValue::Tag {
                    name: *name,
                    qual_path: None,
                    args: vec![].into(),
                })
            } else {
                None
            }
        }),
        Expr::Lit(lit) => Some(match lit {
            ast::Literal::Number(n) => ast::ConstValue::Int(*n as i128),
            ast::Literal::Int(n) => ast::ConstValue::Int(*n as i128),
            ast::Literal::Float(HashFloat(f)) => ast::ConstValue::Float(HashFloat(*f)),
            ast::Literal::String(s) => ast::ConstValue::String(s.clone()),
        }),
        Expr::Bind(bind) => match &bind.value {
            // For `x := 1`, look at the inner literal directly.
            ast::BindValue::Expr(inner) => match &inner.value {
                Expr::Lit(lit) => Some(match lit {
                    ast::Literal::Number(n) => ast::ConstValue::Int(*n as i128),
                    ast::Literal::Int(n) => ast::ConstValue::Int(*n as i128),
                    ast::Literal::Float(HashFloat(f)) => ast::ConstValue::Float(HashFloat(*f)),
                    ast::Literal::String(s) => ast::ConstValue::String(s.clone()),
                }),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    });

    // Extract the span of the symbol name from the AST before lowering,
    // so diagnostics on unknown symbols point at the exact name in source.
    let name_span_id = match &expr.value {
        Expr::FnCall(call) => Some(call.path.span_id),
        Expr::TagCall(tc) => {
            // Qualified tag (e.g. `Maybe.Some`) — use the path span.
            if let Some(qp) = tc.qual_path.as_ref() {
                Some(qp.span_id)
            } else {
                // Bare tag (e.g. `Marker` in `Marker(x)`) — compute a span
                // covering just the tag name from the expression start.
                let expr_span = typed.span_table.get(expr.span_id);
                let name_len = tc.name.as_str().len();
                let span = Span::new(expr_span.start(), expr_span.start() + name_len);
                Some(typed.span_table.insert(span))
            }
        }
        _ => None,
    };

    let kind = lower_expr_kind(typed, expr, scope, env);

    // For compile-time resolved when expressions, propagate the body's const_value.
    let const_val = if let TypedExprKind::When(ref when_expr) = kind {
        if when_expr.arms.len() == 1 && when_expr.subject.is_some() {
            let body_id = match &when_expr.arms[0] {
                TypedWhenArm::Cond { body, .. } | TypedWhenArm::Is { body, .. } => *body,
                TypedWhenArm::Else(body, _) => *body,
            };
            typed.exprs.const_value[body_id.as_usize()]
                .clone()
                .or(const_val)
        } else {
            const_val
        }
    } else {
        const_val
    };

    let expr_id = ExprId(typed.exprs.kind.len() as u32);

    let mut flaws: Vec<Diagnostic> = Vec::new();
    if let TypedExprKind::Reassign { name, .. } = &kind
        && env.constants.contains(name)
    {
        flaws.push(
            Diagnostic::new(
                "type-reassign-constant",
                format!("cannot reassign constant `{}`", name.as_str()),
            )
            .with_arg("name", name.as_str().to_string()),
        );
    }
    check_type_flaws(
        &kind,
        &resolved_ty,
        typed,
        name_span_id,
        scope.locals,
        &env.types,
        &mut flaws,
    );

    // If the expression has a substituted type (from const-generic param resolution),
    // use it instead of the inferred type.
    let final_ty = match &kind {
        TypedExprKind::FnCall {
            substituted_ty: Some(ty),
            ..
        } => ty.clone(),
        _ => resolved_ty,
    };
    typed.exprs.kind.push(kind);
    typed.exprs.ty.push(final_ty);
    typed.exprs.span.push(expr.span_id);
    typed.exprs.const_value.push(const_val);
    typed.exprs.flaws.push(flaws);

    if expr.span_id.is_valid() {
        let span = typed.span_table.get(expr.span_id);
        typed
            .span_to_expr
            .entry(span.start_u32())
            .or_insert(expr_id);
    }

    expr_id
}

fn when_pattern_key(pattern: &ast::TypeExpr) -> String {
    pattern.format_surface()
}

fn when_pattern_is_wildcard(pattern: &ast::TypeExpr) -> bool {
    matches!(pattern, ast::TypeExpr::Nominal(name, _) if name.as_str() == "_")
}

fn is_lowercase_name(name: &Intern<String>) -> bool {
    name.as_str()
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
}

fn pattern_value_def_id(pattern: &ast::TypeExpr) -> Option<DefId> {
    match pattern {
        ast::TypeExpr::Nominal(name, _) if is_lowercase_name(name) && name.as_str() != "_" => {
            Some(DefId(*name))
        }
        ast::TypeExpr::Qualified(path) => Some(super::lower_ty::resolve_fn_call_target(
            &path.value,
            &HashMap::new(),
        )),
        _ => None,
    }
}

fn const_for_def_id(def_id: DefId, typed: &TypedFileAst) -> Option<ast::ConstValue> {
    let Some(bind) = typed.defs.get(&def_id) else {
        let eval_ast = typed.eval_ast.as_ref();
        let source_bind = eval_ast.defs.get(&def_id.0)?;
        let ast::BindValue::Expr(expr) = &source_bind.value else {
            return None;
        };
        let const_binds = const_binds_from_prepared_ast(eval_ast);
        return expr
            .const_value
            .clone()
            .or_else(|| eval_compile_time_expr_public(&expr.value, &const_binds, eval_ast));
    };
    let BindBody::Expr(body) = bind.body else {
        return None;
    };
    typed
        .exprs
        .const_value
        .get(body.as_usize())?
        .clone()
        .or_else(|| match typed.exprs.kind.get(body.as_usize())? {
            TypedExprKind::TagCall {
                variant_id, args, ..
            } => Some(ast::ConstValue::Tag {
                name: variant_id.name,
                qual_path: None,
                args: args
                    .as_ref()
                    .map(|args| {
                        args.iter()
                            .filter_map(|arg| {
                                typed
                                    .exprs
                                    .const_value
                                    .get(arg.as_usize())
                                    .cloned()
                                    .flatten()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            _ => None,
        })
}

/// Try to extract a [`ConstValue`] from a pattern parameter's [`ParameterKind`].
fn param_kind_const(kind: &ParameterKind) -> Option<ast::ConstValue> {
    match kind {
        ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => match &sp.value {
            ast::TypeExpr::Literal(lit, _) => match lit {
                ast::Literal::Int(n) => Some(ast::ConstValue::Int(*n as i128)),
                ast::Literal::Number(n) => Some(ast::ConstValue::Int(*n as i128)),
                ast::Literal::String(s) => Some(ast::ConstValue::String(s.clone())),
                ast::Literal::Float(HashFloat(f)) => Some(ast::ConstValue::Float(HashFloat(*f))),
            },
            _ => None,
        },
        ParameterKind::Generic | ParameterKind::Default(_) => None,
    }
}

pub(crate) fn pattern_value_const(
    pattern: &ast::TypeExpr,
    typed: &TypedFileAst,
) -> Option<ast::ConstValue> {
    // Variant-based resolution for uppercase nominal and qualified patterns
    match pattern {
        // Uppercase nominal: likely a variant tag name (e.g., `True`, `False`)
        ast::TypeExpr::Nominal(name, _) if !is_lowercase_name(name) && name.as_str() != "_" => {
            if typed.variant_map.contains_key(name) {
                return Some(ast::ConstValue::Tag {
                    name: *name,
                    qual_path: None,
                    args: Vec::new().into(),
                });
            }
        }
        // Qualified path: e.g., `Bool.True` → variant "True" of union "Bool"
        ast::TypeExpr::Qualified(path) => {
            if let Some(variant_name) = path.segments.last()
                && typed.variant_map.contains_key(variant_name)
            {
                return Some(ast::ConstValue::Tag {
                    name: *variant_name,
                    qual_path: None,
                    args: Vec::new().into(),
                });
            }
        }
        // Generic variant: e.g. `Some(x)` or `Some(5)` — resolve args if all are concrete
        ast::TypeExpr::Generic { name, params, .. } if typed.variant_map.contains_key(name) => {
            let args: Vec<ast::ConstValue> = params
                .iter()
                .filter_map(|(_, kind)| param_kind_const(kind))
                .collect();
            // Only produce a Tag value if ALL params could be resolved.
            // If any param is a binding (Generic), return None so the
            // reachability check falls through to pattern_matches_public.
            if args.len() == params.len() {
                return Some(ast::ConstValue::Tag {
                    name: *name,
                    qual_path: None,
                    args: args.into(),
                });
            }
        }
        _ => {}
    }
    // Fall back to def-based resolution for lowercase/value patterns
    const_for_def_id(pattern_value_def_id(pattern)?, typed)
}

fn pattern_matches_variant_with_values(
    pattern: &ast::TypeExpr,
    variant_name: Intern<String>,
    typed: &TypedFileAst,
) -> bool {
    if pattern.surface_mangle_name() == variant_name.as_str() {
        return true;
    }
    matches!(
        pattern_value_const(pattern, typed),
        Some(ast::ConstValue::Tag { name, .. }) if name == variant_name
    )
}

fn pattern_matches_const_with_values(
    pattern: &ast::TypeExpr,
    value: &ast::ConstValue,
    typed: &TypedFileAst,
) -> bool {
    pattern_matches_public(pattern, value)
        || pattern_value_const(pattern, typed)
            .as_ref()
            .is_some_and(|pattern_value| pattern_value == value)
}

fn when_is_exhaustive_with_value_patterns(
    subject_ty: Option<&Ty>,
    arms: &[TypedWhenArm],
    typed: &TypedFileAst,
) -> bool {
    let Some(subject_ty) = subject_ty else {
        return false;
    };
    let patterns: Vec<_> = arms
        .iter()
        .filter_map(|arm| match arm {
            TypedWhenArm::Is { pattern, .. } => Some(&pattern.value),
            _ => None,
        })
        .collect();
    if patterns.is_empty() {
        return false;
    }
    if patterns.iter().any(|p| when_pattern_is_wildcard(p)) {
        return true;
    }
    match subject_ty {
        ty if ty.union_literal_values().is_some() => {
            let Some(values) = subject_ty.union_literal_values() else {
                return false;
            };
            values.iter().all(|value| {
                patterns
                    .iter()
                    .any(|pattern| pattern_matches_const_with_values(pattern, value, typed))
            })
        }
        Ty::Union { variants, .. } => variants.iter().all(|(variant_name, _)| {
            patterns
                .iter()
                .any(|pattern| pattern_matches_variant_with_values(pattern, *variant_name, typed))
        }),
        _ => when_is_exhaustive(subject_ty.into(), arms),
    }
}

fn validate_when_arm_shape(
    arms: &[TypedWhenArm],
    has_subject: bool,
    typed: &TypedFileAst,
    flaws: &mut Vec<Diagnostic>,
) {
    let mut seen_else = false;
    let mut seen_is = false;
    let mut seen_cond = false;
    let mut covered_patterns = HashSet::new();
    let mut covered_values: Vec<ast::ConstValue> = Vec::new();
    for arm in arms {
        match arm {
            TypedWhenArm::Else(..) => {
                seen_else = true;
            }
            TypedWhenArm::Cond { .. } => {
                if seen_else {
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                }
                seen_cond = true;
            }
            TypedWhenArm::Is { pattern, .. } => {
                if seen_else {
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                }
                seen_is = true;
                if when_pattern_is_wildcard(&pattern.value) {
                    flaws.push(Diagnostic::new(
                        "type-wildcard-when-pattern",
                        "wildcard `_` is not allowed in when patterns",
                    ));
                }
                // Shape-based duplicate detection
                let key = when_pattern_key(&pattern.value);
                if !covered_patterns.insert(key.clone()) {
                    flaws.push(Diagnostic::new(
                        "type-unreachable-when-arm",
                        "when arm is unreachable",
                    ));
                } else if let Some(cv) = pattern_value_const(&pattern.value, typed) {
                    // Semantic duplicate detection: same resolved value
                    if covered_values.iter().any(|v| v == &cv) {
                        flaws.push(Diagnostic::new(
                            "type-unreachable-when-arm",
                            "when arm is unreachable",
                        ));
                    } else {
                        covered_values.push(cv);
                    }
                }
            }
        }
    }
    if seen_else && !matches!(arms.last(), Some(TypedWhenArm::Else(..))) {
        flaws.push(Diagnostic::new(
            "type-else-not-last",
            "`else` must be the last arm in `when`",
        ));
    }
    if seen_is && (seen_cond || !has_subject) {
        flaws.push(Diagnostic::new(
            "type-mixed-when-forms",
            "cannot mix condition and pattern arms in `when`",
        ));
    }
}

/// Validate named/positional fields in a record-literal TagCall.
///
/// Checks for:
/// - Duplicate field names
/// - Unknown field names
/// - Missing required fields
/// - Type mismatches between arg expressions and declared field types
fn validate_record_fields(
    tag_name: &Intern<String>,
    record_fields: &[(Intern<String>, Box<Ty>)],
    field_names: &[Intern<String>],
    arg_ids: Option<&[ExprId]>,
    typed: &TypedFileAst,
    flaws: &mut Vec<Diagnostic>,
) {
    use std::collections::HashSet;

    let valid_field_set: HashSet<&Intern<String>> = record_fields.iter().map(|(n, _)| n).collect();

    // 1. Duplicate field detection
    let mut seen = HashSet::new();
    for name in field_names {
        if name.as_str().is_empty() {
            continue; // positional arg, handled below
        }
        if !seen.insert(name) {
            flaws.push(
                Diagnostic::new(
                    "type-duplicate-field",
                    format!("field `{}` provided twice", name.as_str()),
                )
                .with_arg("name", name.as_str().to_string()),
            );
        }
    }

    // 2. Unknown field detection
    for name in field_names.iter() {
        if name.as_str().is_empty() {
            flaws.push(Diagnostic::new(
                "type-expected-field-name",
                "expected field name or `name: expr`".to_string(),
            ));
            continue;
        }
        if !valid_field_set.contains(name) {
            flaws.push(
                Diagnostic::new(
                    "type-unknown-field",
                    format!("`{}` has no field `{}`", tag_name.as_str(), name.as_str()),
                )
                .with_arg("name", name.as_str().to_string())
                .with_arg("tag", tag_name.as_str().to_string()),
            );
        }
    }

    // 3. Missing field detection
    let provided_names: HashSet<&Intern<String>> = field_names
        .iter()
        .filter(|n| !n.as_str().is_empty())
        .collect();
    for (field_name, _) in record_fields {
        if !provided_names.contains(field_name) {
            flaws.push(
                Diagnostic::new(
                    "type-missing-field",
                    format!(
                        "missing field `{}` in `{}(...)`",
                        field_name.as_str(),
                        tag_name.as_str()
                    ),
                )
                .with_arg("name", field_name.as_str().to_string())
                .with_arg("tag", tag_name.as_str().to_string()),
            );
        }
    }

    // 4. Type mismatch checks
    if let Some(args) = arg_ids {
        for (i, name) in field_names.iter().enumerate() {
            if name.as_str().is_empty() {
                continue;
            }
            if let Some(arg_id) = args.get(i) {
                let arg_ty = typed.exprs.ty.get(arg_id.as_usize());
                // Find the declared field type
                if let Some((_, field_ty)) = record_fields.iter().find(|(n, _)| *n == *name)
                    && let Some(arg_ty) = arg_ty
                    && types_are_incompatible(arg_ty, field_ty)
                {
                    flaws.push(
                        Diagnostic::new(
                            "type-mismatch",
                            format!("type mismatch for field `{}`", name.as_str(),),
                        )
                        .with_arg("name", name.as_str().to_string()),
                    );
                }
            }
        }
    }
}

/// Check if two types are structurally incompatible (for basic type mismatch detection).
/// Returns `true` if they are definitely incompatible.
fn types_are_incompatible(arg_ty: &Ty, field_ty: &Ty) -> bool {
    match (arg_ty, field_ty) {
        // String literal assigned to Int field
        (Ty::Literal(ConstValue::String(_)), Ty::Int { .. }) => true,
        // Int literal assigned to non-Int field like Bool (union)
        (Ty::Literal(ConstValue::Int(_)), Ty::Union { .. }) => true,
        // String literal assigned to union (Bool is True/False)
        (Ty::Literal(ConstValue::String(_)), Ty::Union { .. }) => true,
        _ => false,
    }
}

fn check_type_flaws(
    kind: &TypedExprKind,
    _ty: &Ty,
    typed: &TypedFileAst,
    name_span_id: Option<SpanId>,
    locals: &HashSet<Intern<String>>,
    local_var_types: &LocalVarTypes,
    flaws: &mut Vec<Diagnostic>,
) {
    match kind {
        TypedExprKind::FnCall { target, args, .. } => {
            check_fn_call_bounds(typed, target, args.as_deref(), flaws);
            let is_known = typed.defs.contains_key(target)
                || typed.fn_return_types.contains_key(target)
                || locals.contains(&target.0)
                || local_var_types.contains_key(&target.0);
            if !is_known {
                let name = target.0.as_str();
                let did_you_mean = closest_name(
                    name,
                    typed
                        .defs
                        .keys()
                        .map(|d| d.0.as_str())
                        .chain(typed.fn_return_types.keys().map(|d| d.0.as_str()))
                        .chain(typed.tags.keys().map(|t| t.0.as_str()))
                        .chain(locals.iter().map(|l| l.as_str()))
                        .chain(local_var_types.keys().map(|t| t.as_str())),
                );
                let mut diag =
                    Diagnostic::new("type-unknown-symbol", format!("unknown symbol `{}`", name))
                        .with_arg("name", name.to_string());
                if let Some(name_span) = name_span_id {
                    diag = diag.at_span_id(name_span, &typed.span_table);
                }
                if let Some(suggestion) = &did_you_mean {
                    diag = diag.with_help(format!("did you mean `{}`?", suggestion));
                }
                flaws.push(diag);
            }
        }
        TypedExprKind::TagCall {
            variant_id,
            field_names,
            args,
            ..
        } => {
            let is_record = is_record_tag_call(variant_id, typed);

            // Unknown symbol: bare tag that isn't a known record
            if variant_id.union.0 == variant_id.name && !is_record {
                let mut diag = Diagnostic::new(
                    "type-unknown-symbol",
                    format!("unknown symbol `{}`", variant_id.name.as_str()),
                )
                .with_arg("name", variant_id.name.as_str().to_string());
                if let Some(name_span) = name_span_id {
                    diag = diag.at_span_id(name_span, &typed.span_table);
                }
                flaws.push(diag);
            }

            // Field validation for record types (shape literals)
            if is_record {
                let tag_id = TagId(variant_id.name);
                if let Some(record_ty) = typed.tag_types.get(&tag_id)
                    && let Ty::Record { fields, .. } = record_ty
                {
                    validate_record_fields(
                        &variant_id.name,
                        fields,
                        field_names,
                        args.as_deref(),
                        typed,
                        flaws,
                    );
                }
            }
        }
        TypedExprKind::Binary { lhs, rhs, .. } => {
            let lhs_ty = typed.exprs.ty.get(lhs.as_usize());
            let rhs_ty = typed.exprs.ty.get(rhs.as_usize());
            if let (Some(lhs_ty), Some(rhs_ty)) = (lhs_ty, rhs_ty) {
                let lhs_is_int = lhs_ty.is_int();
                let rhs_is_int = rhs_ty.is_int();
                let lhs_is_float = lhs_ty.is_float();
                let rhs_is_float = rhs_ty.is_float();
                if (lhs_is_int && rhs_is_float) || (lhs_is_float && rhs_is_int) {
                    flaws.push(Diagnostic::new("type-mismatch", "type mismatch"));
                }
            }
        }
        TypedExprKind::When(when_expr) => {
            validate_when_arm_shape(&when_expr.arms, when_expr.subject.is_some(), typed, flaws);
            let has_else = when_expr
                .arms
                .iter()
                .any(|arm| matches!(arm, TypedWhenArm::Else(..)));
            let subject_ty = when_expr
                .subject
                .and_then(|id| typed.exprs.ty.get(id.as_usize()));
            let exhaustive =
                when_is_exhaustive_with_value_patterns(subject_ty, &when_expr.arms, typed);
            if !has_else && !exhaustive {
                flaws.push(Diagnostic::new(
                    "type-missing-else-arm",
                    "`when` should have an `else` arm",
                ));
            } else if has_else && exhaustive {
                flaws.push(Diagnostic::new(
                    "type-unreachable-else-arm",
                    "`else` arm is unreachable",
                ));
            }
            // Subject-value reachability: if the subject has a known compile-time value,
            // check which arms match and flag unmatched/after-match arms as unreachable.
            if let Some(subject_id) = when_expr.subject
                && let Some(Some(subject_cv)) = typed.exprs.const_value.get(subject_id.as_usize())
            {
                let mut matching_arm_found = false;
                for arm in &when_expr.arms {
                    match arm {
                        TypedWhenArm::Is { pattern, .. } => {
                            if matching_arm_found {
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            } else if let Some(pattern_cv) =
                                pattern_value_const(&pattern.value, typed)
                            {
                                // Pattern resolves to a specific const value
                                if &pattern_cv != subject_cv {
                                    flaws.push(Diagnostic::new(
                                        "type-unreachable-when-arm",
                                        "when arm is unreachable",
                                    ));
                                } else {
                                    matching_arm_found = true;
                                }
                            } else if !crate::analysis::pattern::pattern_matches_with_tag_types(
                                &pattern.value,
                                subject_cv,
                                &typed.tag_types,
                            ) {
                                // Pattern can be statically compared against subject value
                                // (handles literal, InRange, uppercase variant patterns)
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-when-arm",
                                    "when arm is unreachable",
                                ));
                            } else {
                                matching_arm_found = true;
                            }
                        }
                        TypedWhenArm::Else(..) => {
                            if matching_arm_found {
                                flaws.push(Diagnostic::new(
                                    "type-unreachable-else-arm",
                                    "`else` arm is unreachable",
                                ));
                            }
                        }
                        TypedWhenArm::Cond { .. } => {}
                    }
                }
            }

            // Gin has no builtin types — only ADTs — so any type may be used as
            // a when condition without restriction.
            for arm in &when_expr.arms {
                let TypedWhenArm::Cond { condition, .. } = arm else {
                    continue;
                };
                let _ = typed.exprs.ty.get(condition.as_usize());
            }
        }
        TypedExprKind::Destructure {
            value,
            field_bindings,
            ..
        } => {
            // Validate that each field name exists on the base record type
            let base_ty = typed.exprs.ty.get(value.as_usize());
            if let Some(Ty::Record { fields, .. }) = base_ty {
                for (field_name, _bind_name) in field_bindings {
                    if !fields.iter().any(|(n, _)| n == field_name) {
                        flaws.push(
                            Diagnostic::new(
                                "type-unknown-field",
                                format!("record has no field `{}`", field_name.as_str()),
                            )
                            .with_arg("name", field_name.as_str().to_string()),
                        );
                    }
                }
            }
        }
        TypedExprKind::RecordSet { base, field, .. } => {
            // Check that the field exists on the base record type
            let base_ty = typed.exprs.ty.get(base.as_usize());
            if let Some(Ty::Record { fields, .. }) = base_ty
                && !fields.iter().any(|(n, _)| n.as_str() == field.as_str())
            {
                flaws.push(
                    Diagnostic::new(
                        "type-unknown-field",
                        format!("record has no field `{}`", field.as_str()),
                    )
                    .with_arg("name", field.as_str().to_string()),
                );
            }
        }
        _ => {}
    }
}
