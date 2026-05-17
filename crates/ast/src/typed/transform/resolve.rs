//! Stage 2: Resolve — Lower parse expressions to the typed arena, attach type flaws.
//!
//! This stage walks the parse-tree expressions, infers/resolves their types,
//! converts them to [`TypedExprKind`], pushes them into the expression arena,
//! and attaches type-check flaws.

use internment::Intern;
use std::collections::{HashMap, HashSet};

use super::{ParseAst, TransformCtx};
use crate::prelude::*;
use crate::ty::Ty;
use crate::typed::{BindBody, DefId, ExprId, TagId, TypedExprKind, TypedFileAst, VariantId};
use diagnostic::TypeSymptom;

/// Computes the Levenshtein distance between two strings.
fn edit_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    // Bails early on large length differences.
    let max_dist: usize = 2;
    if a_len.abs_diff(b_len) > max_dist {
        return max_dist + 1;
    }

    // Use two-row technique for O(min(a_len, b_len)) memory.
    let (shorter, longer) = if a_len < b_len { (a, b) } else { (b, a) };
    let s_len = shorter.chars().count();
    let l_chars: Vec<char> = longer.chars().collect();

    let mut prev: Vec<usize> = (0..=s_len).collect();
    let mut curr: Vec<usize> = vec![0; s_len + 1];

    for (i, lc) in l_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, sc) in shorter.chars().enumerate() {
            let cost = if lc == &sc { 0 } else { 1 };
            curr[j + 1] =
                std::cmp::min(std::cmp::min(curr[j] + 1, prev[j + 1] + 1), prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[s_len]
}

/// Returns the closest matching name from `candidates` within edit distance ≤ 2,
/// or `None` if no candidate is close enough.
fn closest_name<'a>(target: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &'a str)> = None;
    for candidate in candidates {
        if candidate == target {
            continue;
        }
        let dist = edit_distance(target, candidate);
        if dist <= 2 {
            match best {
                Some((prev_dist, _)) if dist < prev_dist => {
                    best = Some((dist, candidate));
                }
                None => {
                    best = Some((dist, candidate));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, name)| name.to_string())
}

/// Walks all bind bodies and top-level expressions in the `ParseAst`,
/// converts parse-tree `Expr` nodes to `TypedExprKind`, pushes them into
/// the expression arena, and attaches type-check flaws.
pub fn stage_resolve(typed: &mut TypedFileAst, parse_ast: &ParseAst, _ctx: &TransformCtx) {
    // Build a combined tag-type lookup: Intern<String> → Ty
    let tag_types: HashMap<Intern<String>, Ty> = typed
        .tag_types
        .iter()
        .map(|(tid, ty)| (tid.0, ty.clone()))
        .collect();

    // Collect variant_map reference before any mutable borrows.
    let variant_map: crate::analysis::VariantMap = typed.variant_map.clone();

    // First pass: lower all expressions, collecting body assignments.
    // We avoid borrowing typed.defs while borrowing typed.exprs (arena).
    struct DefBodyAssign {
        def_id: DefId,
        body: BindBody,
    }

    let def_ids: Vec<DefId> = typed.defs.keys().copied().collect();

    // Pre-collect receiver types and param names to pass during expression
    // lowering without holding an immutable borrow on typed.defs.
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
            let params: HashSet<Intern<String>> = typed
                .defs
                .get(def_id)
                .map(|b| b.params.iter().map(|(n, _)| *n).collect())
                .unwrap_or_default();
            (*def_id, params)
        })
        .collect();

    let mut assignments: Vec<DefBodyAssign> = Vec::new();

    for def_id in &def_ids {
        let Some(bind) = parse_ast.defs.get(&def_id.0) else {
            continue;
        };

        let receiver_type = receiver_types.get(def_id).and_then(|r| r.as_ref());
        let empty_locals = HashSet::new();
        let locals: &HashSet<Intern<String>> = param_sets.get(def_id).unwrap_or(&empty_locals);

        let body = match &bind.value {
            BindValue::Expr(typed_expr) => {
                let id = lower_typed_expr(
                    typed,
                    typed_expr.as_ref(),
                    &tag_types,
                    &variant_map,
                    receiver_type,
                    locals,
                );
                BindBody::Expr(id)
            }
            BindValue::Body { exprs, ret } => {
                let mut lowered_exprs: Vec<ExprId> = Vec::new();
                for expr in exprs {
                    lowered_exprs.push(lower_typed_expr(
                        typed,
                        expr,
                        &tag_types,
                        &variant_map,
                        receiver_type,
                        locals,
                    ));
                }
                let ret_id = ret.value.as_ref().map(|ret_expr| {
                    lower_typed_expr(
                        typed,
                        ret_expr,
                        &tag_types,
                        &variant_map,
                        receiver_type,
                        locals,
                    )
                });
                BindBody::Body {
                    exprs: lowered_exprs,
                    ret: ret_id,
                }
            }
            BindValue::Extern => BindBody::Extern,
        };
        assignments.push(DefBodyAssign {
            def_id: *def_id,
            body,
        });
    }

    // Lower top-level expressions.
    let empty_locals = HashSet::new();
    for (expr, span_id) in &parse_ast.exprs {
        let wrapped = Typed::infer(expr.clone(), *span_id);
        let expr_id = lower_typed_expr(
            typed,
            &wrapped,
            &tag_types,
            &variant_map,
            None,
            &empty_locals,
        );
        typed.root_exprs.push(expr_id);
    }

    // Second pass: assign bodies to defs (separate borrow from arena).
    for assign in assignments {
        if let Some(typed_bind) = typed.defs.get_mut(&assign.def_id) {
            typed_bind.body = assign.body;
        }
    }
}

fn lower_typed_expr(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &crate::analysis::VariantMap,
    receiver_type: Option<&Ty>,
    locals: &HashSet<Intern<String>>,
) -> ExprId {
    let resolved_ty = resolve_expr_type(expr, tag_types, variant_map, receiver_type);
    let const_val = expr.const_value.clone();

    let kind = lower_expr_kind(typed, expr, tag_types, variant_map, receiver_type, locals);
    let expr_id = ExprId(typed.exprs.kind.len() as u32);

    let mut flaws: Vec<diagnostic::TypeSymptom> = Vec::new();
    check_type_flaws(&kind, &resolved_ty, typed, locals, &mut flaws);

    typed.exprs.kind.push(kind);
    typed.exprs.ty.push(resolved_ty);
    typed.exprs.span.push(expr.span_id);
    typed.exprs.const_value.push(const_val);
    typed.exprs.flaws.push(flaws);
    typed.exprs.flow.push(None);

    let span = typed.span_table.get(expr.span_id);
    typed
        .span_to_expr
        .entry(span.start as u32)
        .or_insert(expr_id);

    expr_id
}

fn check_type_flaws(
    kind: &TypedExprKind,
    _ty: &Ty,
    typed: &TypedFileAst,
    locals: &HashSet<Intern<String>>,
    flaws: &mut Vec<diagnostic::TypeSymptom>,
) {
    match kind {
        TypedExprKind::FnCall { target, .. } => {
            let is_known = typed.defs.contains_key(target)
                || typed.fn_return_types.contains_key(target)
                || locals.contains(&target.0);
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
                        .chain(locals.iter().map(|l| l.as_str())),
                );
                flaws.push(TypeSymptom::UnknownBinding {
                    name: name.to_string(),
                    did_you_mean,
                });
            }
        }
        TypedExprKind::TagCall { variant_id, .. }
            // If the variant name is the same as the union name, we couldn't resolve it.
            if variant_id.union.0 == variant_id.name => {
                flaws.push(TypeSymptom::UnknownTag {
                    name: variant_id.name.as_str().to_string(),
                });
            }
        TypedExprKind::Binary { lhs, rhs, .. } => {
            let lhs_ty = typed.exprs.ty.get(lhs.as_usize());
            let rhs_ty = typed.exprs.ty.get(rhs.as_usize());
            if let (Some(lhs_ty), Some(rhs_ty)) = (lhs_ty, rhs_ty) {
                let lhs_is_int = lhs_ty.is_int();
                let rhs_is_int = rhs_ty.is_int();
                let lhs_is_float = lhs_ty.is_float();
                let rhs_is_float = rhs_ty.is_float();
                // Mismatch when one operand is int and the other is float.
                if (lhs_is_int && rhs_is_float) || (lhs_is_float && rhs_is_int) {
                    flaws.push(TypeSymptom::Mismatch);
                }
            }
        }
        TypedExprKind::When(when_expr) => {
            // Check for missing Else arm.
            let has_else = when_expr
                .arms
                .iter()
                .any(|arm| matches!(arm, crate::typed::TypedWhenArm::Else(..)));
            if !has_else {
                flaws.push(TypeSymptom::MissingElseArm);
            }
            // Check for non-bool conditions.
            for arm in &when_expr.arms {
                let crate::typed::TypedWhenArm::Cond { condition, .. } = arm else { continue };
                let Some(cond_ty) = typed.exprs.ty.get(condition.as_usize()) else { continue };
                    if !cond_ty.is_bool_like() {
                        flaws.push(TypeSymptom::ConditionNotBool {
                            got: crate::typed::format_ty_for_hover(cond_ty),
                        });
                    }
            }
        }
        _ => {}
    }
}

fn lower_expr_kind(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &crate::analysis::VariantMap,
    receiver_type: Option<&Ty>,
    locals: &HashSet<Intern<String>>,
) -> TypedExprKind {
    match &expr.value {
        Expr::Lit(lit) => TypedExprKind::Lit(lit.clone()),

        Expr::Binary(binary) => {
            let lhs = lower_typed_expr(
                typed,
                &binary.lhs,
                tag_types,
                variant_map,
                receiver_type,
                locals,
            );
            let rhs = lower_typed_expr(
                typed,
                &binary.rhs,
                tag_types,
                variant_map,
                receiver_type,
                locals,
            );
            TypedExprKind::Binary {
                op: binary.op.clone(),
                lhs,
                rhs,
            }
        }

        Expr::FnCall(fn_call) => {
            let target = resolve_fn_call_target(&fn_call.path.value, tag_types);
            let args = fn_call.args.as_ref().map(|args| {
                args.iter()
                    .map(|a| {
                        lower_typed_expr(typed, a, tag_types, variant_map, receiver_type, locals)
                    })
                    .collect()
            });
            TypedExprKind::FnCall { target, args }
        }

        Expr::TagCall(tag_call) => {
            let variant_id = resolve_tag_call_variant(tag_call, variant_map);
            let discriminant = resolve_discriminant(&variant_id, variant_map);
            let args = if tag_call.args.is_empty() {
                None
            } else {
                Some(
                    tag_call
                        .args
                        .iter()
                        .map(|a| {
                            lower_typed_expr(
                                typed,
                                a,
                                tag_types,
                                variant_map,
                                receiver_type,
                                locals,
                            )
                        })
                        .collect(),
                )
            };
            TypedExprKind::TagCall {
                variant_id,
                discriminant,
                args,
            }
        }

        Expr::AnonymousTag(name) => {
            let candidates = variant_map.get(name).cloned().unwrap_or_default();
            let variant_id = candidates
                .first()
                .map(|(union, _, _)| VariantId {
                    union: TagId(*union),
                    name: *name,
                })
                .unwrap_or_else(|| VariantId {
                    union: TagId(*name),
                    name: *name,
                });
            let discriminant = resolve_discriminant(&variant_id, variant_map);
            TypedExprKind::TagCall {
                variant_id,
                discriminant,
                args: None,
            }
        }

        Expr::Bind(local_bind) => {
            // Local bind (e.g., `val x: 42` inside a function body).
            // Extract the body expression from the bind value.
            match &local_bind.value {
                BindValue::Expr(typed_expr) => {
                    let body = lower_typed_expr(
                        typed,
                        typed_expr,
                        tag_types,
                        variant_map,
                        receiver_type,
                        locals,
                    );
                    TypedExprKind::Bind {
                        name: local_bind.name,
                        body,
                    }
                }
                BindValue::Body { exprs, ret } => {
                    // For multi-expr bodies, lower the last expression as the value.
                    if let Some(last) = exprs.last() {
                        let body = lower_typed_expr(
                            typed,
                            last,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        TypedExprKind::Bind {
                            name: local_bind.name,
                            body,
                        }
                    } else if let Some(ret_expr) = &ret.value {
                        let body = lower_typed_expr(
                            typed,
                            ret_expr,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        TypedExprKind::Bind {
                            name: local_bind.name,
                            body,
                        }
                    } else {
                        TypedExprKind::Lit(Literal::Number(0))
                    }
                }
                BindValue::Extern => TypedExprKind::Lit(Literal::Number(0)),
            }
        }

        Expr::When(when_expr) => {
            let subject = when_expr
                .subject
                .as_ref()
                .map(|s| lower_typed_expr(typed, s, tag_types, variant_map, receiver_type, locals));
            let arms: Vec<crate::typed::TypedWhenArm> = when_expr
                .arms
                .iter()
                .map(|arm| match arm {
                    WhenArm::Cond {
                        condition,
                        body,
                        arm_span,
                    } => {
                        let cond_id = lower_typed_expr(
                            typed,
                            condition,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        let body_id = lower_typed_expr(
                            typed,
                            body,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        crate::typed::TypedWhenArm::Cond {
                            condition: cond_id,
                            body: body_id,
                            arm_span: *arm_span,
                        }
                    }
                    WhenArm::Is {
                        pattern,
                        body,
                        arm_span,
                    } => {
                        let body_id = lower_typed_expr(
                            typed,
                            body,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        crate::typed::TypedWhenArm::Is {
                            pattern: pattern.clone(),
                            body: body_id,
                            arm_span: *arm_span,
                        }
                    }
                    WhenArm::Else(body, arm_span) => {
                        let body_id = lower_typed_expr(
                            typed,
                            body,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        );
                        crate::typed::TypedWhenArm::Else(body_id, *arm_span)
                    }
                })
                .collect();
            TypedExprKind::When(crate::typed::TypedWhenExpr {
                subject,
                arms,
                body_span: when_expr.body_span,
            })
        }
        Expr::If(if_expr) => {
            let (condition, then_body, else_body) = match &if_expr.condition {
                IfCondition::Bool(cond) => {
                    let cond_id = lower_typed_expr(
                        typed,
                        cond,
                        tag_types,
                        variant_map,
                        receiver_type,
                        locals,
                    );
                    let body_id = if let Some(last) = if_expr.body.last() {
                        lower_typed_expr(typed, last, tag_types, variant_map, receiver_type, locals)
                    } else if let Some(ret_val) = &if_expr.ret.value {
                        lower_typed_expr(
                            typed,
                            ret_val,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        )
                    } else {
                        lower_typed_expr(typed, expr, tag_types, variant_map, receiver_type, locals)
                    };
                    (crate::typed::TypedIfCondition::Bool(cond_id), body_id, None)
                }
                IfCondition::Pattern { subject, pattern } => {
                    let subj_id = lower_typed_expr(
                        typed,
                        subject,
                        tag_types,
                        variant_map,
                        receiver_type,
                        locals,
                    );
                    let body_id = if let Some(last) = if_expr.body.last() {
                        lower_typed_expr(typed, last, tag_types, variant_map, receiver_type, locals)
                    } else if let Some(ret_val) = &if_expr.ret.value {
                        lower_typed_expr(
                            typed,
                            ret_val,
                            tag_types,
                            variant_map,
                            receiver_type,
                            locals,
                        )
                    } else {
                        lower_typed_expr(typed, expr, tag_types, variant_map, receiver_type, locals)
                    };
                    (
                        crate::typed::TypedIfCondition::Pattern {
                            subject: subj_id,
                            pattern: pattern.clone(),
                        },
                        body_id,
                        None,
                    )
                }
            };
            TypedExprKind::If(crate::typed::TypedIfExpr {
                condition,
                then_body,
                else_body,
                body_span: if_expr.body_span,
            })
        }
        Expr::Loop(loop_expr) => {
            match loop_expr {
                Loop::While(while_loop) => {
                    let cond_id = lower_typed_expr(
                        typed,
                        &while_loop.cond,
                        tag_types,
                        variant_map,
                        receiver_type,
                        locals,
                    );
                    // While body is a block — lower the last expression as the body.
                    let body_id = while_loop
                        .exprs
                        .last()
                        .map(|last| {
                            lower_typed_expr(
                                typed,
                                last,
                                tag_types,
                                variant_map,
                                receiver_type,
                                locals,
                            )
                        })
                        .unwrap_or_else(|| {
                            let id = typed.exprs.kind.len() as u32;
                            typed
                                .exprs
                                .kind
                                .push(TypedExprKind::Lit(Literal::Number(0)));
                            typed.exprs.ty.push(crate::ty::Ty::Unit);
                            typed.exprs.span.push(SpanId::INVALID);
                            typed.exprs.const_value.push(None);
                            typed.exprs.flaws.push(Vec::new());
                            typed.exprs.flow.push(None);
                            ExprId(id)
                        });
                    TypedExprKind::Loop(crate::typed::TypedLoop {
                        kind: crate::typed::TypedLoopKind::While { condition: cond_id },
                        body: body_id,
                        keyword_span: while_loop.keyword_span,
                    })
                }
                Loop::ForIn(for_in) => {
                    let iter_id = lower_typed_expr(
                        typed,
                        &for_in.iter,
                        tag_types,
                        variant_map,
                        receiver_type,
                        locals,
                    );
                    let body_id = for_in
                        .exprs
                        .last()
                        .map(|last| {
                            lower_typed_expr(
                                typed,
                                last,
                                tag_types,
                                variant_map,
                                receiver_type,
                                locals,
                            )
                        })
                        .unwrap_or_else(|| {
                            let id = typed.exprs.kind.len() as u32;
                            typed
                                .exprs
                                .kind
                                .push(TypedExprKind::Lit(Literal::Number(0)));
                            typed.exprs.ty.push(crate::ty::Ty::Unit);
                            typed.exprs.span.push(SpanId::INVALID);
                            typed.exprs.const_value.push(None);
                            typed.exprs.flaws.push(Vec::new());
                            typed.exprs.flow.push(None);
                            ExprId(id)
                        });
                    // Extract variable name from the for-in pattern.
                    let var_name = match &for_in.pat.value {
                        Expr::Bind(b) => b.name.as_str().to_string(),
                        _ => Intern::new("it".to_string()).as_str().to_string(),
                    };
                    TypedExprKind::Loop(crate::typed::TypedLoop {
                        kind: crate::typed::TypedLoopKind::ForIn {
                            variable: Intern::new(var_name),
                            iterable: iter_id,
                        },
                        body: body_id,
                        keyword_span: for_in.keyword_span,
                    })
                }
            }
        }

        Expr::SelfRef => TypedExprKind::SelfRef {
            target: DefId(Intern::new("self".to_string())),
        },

        Expr::FormatString(fs) => TypedExprKind::FormatString(fs.clone()),

        Expr::Range(range) => {
            let start = lower_typed_expr(
                typed,
                &range.start,
                tag_types,
                variant_map,
                receiver_type,
                locals,
            );
            let end = lower_typed_expr(
                typed,
                &range.end,
                tag_types,
                variant_map,
                receiver_type,
                locals,
            );
            TypedExprKind::Range { start, end }
        }

        Expr::TupleAlloc { init, size } => {
            let init_id =
                lower_typed_expr(typed, init, tag_types, variant_map, receiver_type, locals);
            TypedExprKind::TupleAlloc {
                init: init_id,
                size: *size,
            }
        }

        Expr::TupleGet { base, index } => {
            let base_id =
                lower_typed_expr(typed, base, tag_types, variant_map, receiver_type, locals);
            TypedExprKind::TupleGet {
                base: base_id,
                index: *index,
            }
        }

        Expr::TupleSet { base, index, value } => {
            let base_id =
                lower_typed_expr(typed, base, tag_types, variant_map, receiver_type, locals);
            let value_id =
                lower_typed_expr(typed, value, tag_types, variant_map, receiver_type, locals);
            TypedExprKind::TupleSet {
                base: base_id,
                index: *index,
                value: value_id,
            }
        }

        Expr::Cast {
            expr: cast_expr,
            ty,
        } => {
            let expr_id = lower_typed_expr(
                typed,
                cast_expr,
                tag_types,
                variant_map,
                receiver_type,
                locals,
            );
            let cast_ty = tag_types.get(ty).cloned().unwrap_or(Ty::Opaque(*ty));
            TypedExprKind::Cast {
                expr: expr_id,
                ty: cast_ty,
            }
        }

        Expr::BufGet { buf, index } => {
            let buf_id =
                lower_typed_expr(typed, buf, tag_types, variant_map, receiver_type, locals);
            let index_id =
                lower_typed_expr(typed, index, tag_types, variant_map, receiver_type, locals);
            TypedExprKind::BufGet {
                buf: buf_id,
                index: index_id,
            }
        }

        Expr::BufSet { buf, index, value } => {
            let buf_id =
                lower_typed_expr(typed, buf, tag_types, variant_map, receiver_type, locals);
            let index_id =
                lower_typed_expr(typed, index, tag_types, variant_map, receiver_type, locals);
            let value_id =
                lower_typed_expr(typed, value, tag_types, variant_map, receiver_type, locals);
            TypedExprKind::BufSet {
                buf: buf_id,
                index: index_id,
                value: value_id,
            }
        }

        Expr::TakePtr(inner) => TypedExprKind::TakePtr(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),
        Expr::TakeRef(inner) => TypedExprKind::TakeRef(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),
        Expr::Deref(inner) => TypedExprKind::Deref(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),
        Expr::Negate(inner) => TypedExprKind::Negate(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),
        Expr::MutArg(inner) => TypedExprKind::MutArg(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),
        Expr::OwnArg(inner) => TypedExprKind::OwnArg(lower_typed_expr(
            typed,
            inner,
            tag_types,
            variant_map,
            receiver_type,
            locals,
        )),

        Expr::TupleLit(items) => {
            let lowered: Vec<ExprId> = items
                .iter()
                .map(|item| {
                    lower_typed_expr(typed, item, tag_types, variant_map, receiver_type, locals)
                })
                .collect();
            TypedExprKind::TupleLit(lowered)
        }

        Expr::List(items) => {
            let lowered: Vec<ExprId> = items
                .iter()
                .map(|item| {
                    lower_typed_expr(typed, item, tag_types, variant_map, receiver_type, locals)
                })
                .collect();
            TypedExprKind::List(lowered)
        }

        Expr::Asm(asm) => TypedExprKind::Asm(asm.clone()),

        // Type-level expressions desugared away
        Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
            TypedExprKind::Lit(Literal::Number(0))
        }
    }
}

fn resolve_expr_type(
    expr: &Typed<Expr>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &crate::analysis::VariantMap,
    receiver_type: Option<&Ty>,
) -> Ty {
    if let Some(resolved) = expr.resolved_ty() {
        return resolved.clone();
    }
    match &expr.value {
        Expr::Lit(lit) => lit_to_ty(lit),
        Expr::Binary(bin) => {
            let lhs_ty = resolve_expr_type(&bin.lhs, tag_types, variant_map, receiver_type);
            if bin.op.is_comparison() {
                Ty::Bool
            } else {
                lhs_ty
            }
        }
        Expr::FnCall(_) => Ty::Opaque(Intern::new("infer".to_string())),
        Expr::TagCall(tag_call) => resolve_tag_call_type(&tag_call.name, tag_types, variant_map),
        Expr::AnonymousTag(name) => tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name)),
        Expr::Bind(local_bind) => match &local_bind.value {
            BindValue::Expr(e) => resolve_expr_type(e, tag_types, variant_map, receiver_type),
            BindValue::Body { exprs, ret } => exprs
                .last()
                .map(|e| resolve_expr_type(e, tag_types, variant_map, receiver_type))
                .or_else(|| {
                    ret.value
                        .as_ref()
                        .map(|e| resolve_expr_type(e, tag_types, variant_map, receiver_type))
                })
                .unwrap_or(Ty::Unit),
            BindValue::Extern => Ty::Unit,
        },
        Expr::When(when_expr) => when_expr
            .arms
            .first()
            .map(|arm| match arm {
                WhenArm::Cond { body, .. } | WhenArm::Is { body, .. } => {
                    resolve_expr_type(body, tag_types, variant_map, receiver_type)
                }
                WhenArm::Else(body, _) => {
                    resolve_expr_type(body, tag_types, variant_map, receiver_type)
                }
            })
            .unwrap_or(Ty::Opaque(Intern::new("infer".to_string()))),
        Expr::If(if_expr) => if_expr
            .body
            .first()
            .map(|e| resolve_expr_type(e, tag_types, variant_map, receiver_type))
            .unwrap_or(Ty::Unit),
        Expr::Loop(_) => Ty::Unit,
        Expr::SelfRef => receiver_type
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(Intern::new("self".to_string()))),
        Expr::FormatString(_) => Ty::Opaque(Intern::new("Str".to_string())),
        Expr::Range(_) => Ty::Array {
            elem: Box::new(Ty::Int {
                width: 64,
                signed: true,
                value: None,
            }),
            size: 0,
        },
        Expr::TupleAlloc { init, .. } => {
            let elem_ty = resolve_expr_type(init, tag_types, variant_map, receiver_type);
            Ty::Array {
                elem: Box::new(elem_ty),
                size: 0,
            }
        }
        Expr::TupleGet { base, .. } => extract_element_type(&resolve_expr_type(
            base,
            tag_types,
            variant_map,
            receiver_type,
        )),
        Expr::TupleSet { value, .. } => {
            resolve_expr_type(value, tag_types, variant_map, receiver_type)
        }
        Expr::Cast { ty, .. } => tag_types.get(ty).cloned().unwrap_or(Ty::Opaque(*ty)),
        Expr::BufGet { buf, .. } => extract_element_type(&resolve_expr_type(
            buf,
            tag_types,
            variant_map,
            receiver_type,
        )),
        Expr::BufSet { value, .. } => {
            resolve_expr_type(value, tag_types, variant_map, receiver_type)
        }
        Expr::TakePtr(inner) => Ty::Ptr {
            inner: Box::new(resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
            )),
        },
        Expr::TakeRef(inner) => Ty::Ref {
            inner: Box::new(resolve_expr_type(
                inner,
                tag_types,
                variant_map,
                receiver_type,
            )),
        },
        Expr::Deref(inner) => {
            let inner_ty = resolve_expr_type(inner, tag_types, variant_map, receiver_type);
            match &inner_ty {
                Ty::Ptr { inner } | Ty::Ref { inner } => *inner.clone(),
                _ => Ty::Opaque(Intern::new("infer".to_string())),
            }
        }
        Expr::Negate(inner) => resolve_expr_type(inner, tag_types, variant_map, receiver_type),
        Expr::MutArg(inner) | Expr::OwnArg(inner) => {
            resolve_expr_type(inner, tag_types, variant_map, receiver_type)
        }
        Expr::TupleLit(items) => Ty::Tuple(
            items
                .iter()
                .map(|item| resolve_expr_type(item, tag_types, variant_map, receiver_type))
                .collect(),
        ),
        Expr::List(items) => items
            .first()
            .map(|e| resolve_expr_type(e, tag_types, variant_map, receiver_type))
            .unwrap_or(Ty::Opaque(Intern::new("List".to_string()))),
        Expr::Asm(_) => Ty::Unit,
        Expr::TypeNominal(name) | Expr::TypeGeneric { name, .. } => {
            tag_types.get(name).cloned().unwrap_or(Ty::Opaque(*name))
        }
        Expr::TypeQualified(path) => {
            let last = path.segments.last().copied().unwrap_or(path.root);
            tag_types.get(&last).cloned().unwrap_or(Ty::Opaque(last))
        }
    }
}

fn lit_to_ty(lit: &Literal) -> Ty {
    match lit {
        Literal::Number(_) => Ty::Int {
            width: 64,
            signed: true,
            value: None,
        },
        Literal::Float(_) => Ty::Float { value: None },
        Literal::Int(_) => Ty::Int {
            width: 64,
            signed: false,
            value: None,
        },
        Literal::String(_) => Ty::Opaque(Intern::new("Str".to_string())),
    }
}

fn extract_element_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Array { elem, .. } => *elem.clone(),
        Ty::Tuple(fields) => fields.first().cloned().unwrap_or(Ty::Unit),
        _ => Ty::Opaque(Intern::new("infer".to_string())),
    }
}

fn resolve_fn_call_target(
    path: &crate::path::ModPath,
    _tag_types: &HashMap<Intern<String>, Ty>,
) -> DefId {
    let fq_name = if path.segments.is_empty() {
        path.root
    } else {
        let mut parts: Vec<&str> = vec![path.root.as_str()];
        for seg in &path.segments {
            parts.push(seg.as_str());
        }
        Intern::new(parts.join("."))
    };
    DefId(fq_name)
}

fn resolve_tag_call_variant(
    tag_call: &crate::expr::TagCall,
    variant_map: &crate::analysis::VariantMap,
) -> VariantId {
    let name = tag_call.name;
    if let Some(qual_path) = &tag_call.qual_path {
        // `qual_path` includes the variant as the last segment.
        // For `Bool.False`: root="Bool", segments=["False"] → union is root.
        // For `Mod.Bool.False`: root="Mod", segments=["Bool","False"] → union is segments[len-2].
        let qual_name = if qual_path.segments.len() > 1 {
            qual_path.segments[qual_path.segments.len() - 2]
        } else {
            qual_path.root
        };
        return VariantId {
            union: TagId(qual_name),
            name,
        };
    }
    if let Some(candidates) = variant_map.get(&name) {
        if let Some((union_name, _, _)) = candidates.first() {
            return VariantId {
                union: TagId(*union_name),
                name,
            };
        }
    }
    VariantId {
        union: TagId(name),
        name,
    }
}

fn resolve_discriminant(
    variant_id: &VariantId,
    variant_map: &crate::analysis::VariantMap,
) -> usize {
    if let Some(candidates) = variant_map.get(&variant_id.name) {
        for (union_name, disc, _) in candidates {
            if *union_name == variant_id.union.0 {
                return *disc;
            }
        }
    }
    0
}

fn resolve_tag_call_type(
    name: &Intern<String>,
    tag_types: &HashMap<Intern<String>, Ty>,
    variant_map: &crate::analysis::VariantMap,
) -> Ty {
    if let Some(candidates) = variant_map.get(name)
        && let Some((union_name, _, _)) = candidates.first()
        && let Some(ty) = tag_types.get(union_name)
    {
        return ty.clone();
    }
    Ty::Opaque(*name)
}
