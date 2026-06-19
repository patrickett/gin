//! Expression-kind lowering — the main match that converts each [`Expr`]
//! variant to [`TypedExprKind`].
//!
//! This is the central dispatch of the lowering stage. It delegates to
//! sibling modules for tag resolution, type resolution, and when-arm
//! lowering.

use ast::prelude::*;
use ast::span::{SpanId, Spanned, SubSpan};
use internment::Intern;

use crate::ty::Ty;

use crate::compile_time_trait::{CompileTimeTraitRegistry, trait_field_for_ty};
use crate::typed::{
    DefId, ExprId, TypedExprKind, TypedFileAst, TypedIfExpr, TypedLoop, TypedLoopKind,
    TypedWhenExpr,
};

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, LocalEnv};
use super::lower_tag::{resolve_discriminant, resolve_tag_call_variant};
use super::lower_ty::{annotate_literal_union_literal, bind_explicit_ty, resolve_fn_call_target};
use super::lower_when::lower_all_when_arms;

fn happy_pattern_for_subject(
    typed: &TypedFileAst,
    subject_ty: &Ty,
    span_id: SpanId,
) -> Option<Box<Spanned<TypeExpr>>> {
    let registry = CompileTimeTraitRegistry {
        blanket_impls: typed.blanket_impls.clone(),
        imported_traits: typed.imported_trait_names.clone(),
        eval_ast: typed.eval_ast.clone(),
    };
    let ast::ConstValue::Tag {
        name, qual_path, ..
    } = trait_field_for_ty("Happy", "value", subject_ty, typed, &registry)?
    else {
        return None;
    };

    let value = if let Some(qual_path) = qual_path {
        let mut parts = qual_path.split('.');
        let root = Intern::new(parts.next()?.to_string());
        let mut segments: Vec<Intern<String>> = parts.map(|p| Intern::new(p.to_string())).collect();
        segments.push(name);
        TypeExpr::Qualified(Spanned {
            value: ast::ModPath::new(root, segments),
            span_id,
        })
    } else {
        TypeExpr::Nominal(name, span_id)
    };
    Some(Box::new(Spanned { value, span_id }))
}

/// Extract field names from TagCall args.
///
/// Named args (`x: expr`) carry the field name in an `Expr::Bind` wrapper.
/// Shorthand args (`x`) carry the field name as the path root of an `Expr::FnCall`.
/// Positional/expression args (bare literals or compound expressions without a
/// field name) produce an empty `Intern` — they will be caught by validation.
pub(crate) fn extract_tag_call_field_names(args: &[Typed<Expr>]) -> Vec<Intern<String>> {
    args.iter()
        .map(|a| match &a.value {
            Expr::Bind(b) => b.name,
            Expr::FnCall(fc) => fc.path.root,
            _ => Intern::new(String::new()),
        })
        .collect()
}

/// Lower a `TagCall` argument. Named record fields are often encoded as `Expr::Bind`
/// carriers; those are not real locals and must not enter flow analysis.
pub(crate) fn lower_tag_call_arg(
    typed: &mut TypedFileAst,
    arg: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
) -> ExprId {
    if let Expr::Bind(b) = &arg.value
        && b.params.is_none()
        && let BindValue::Expr(inner) = &b.value
    {
        return lower_typed_expr(typed, inner, &scope.child(), env);
    }
    lower_typed_expr(typed, arg, &scope.child(), env)
}

/// Convert a parse-tree [`Expr`] to a [`TypedExprKind`] by recursively
/// lowering sub-expressions and resolving types.
pub(crate) fn lower_expr_kind(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
) -> TypedExprKind {
    match &expr.value {
        Expr::Lit(lit) => TypedExprKind::Lit(lit.clone()),

        Expr::Binary(binary) => {
            let lhs = lower_typed_expr(typed, &binary.lhs, &scope.child(), env);
            let rhs = lower_typed_expr(typed, &binary.rhs, &scope.child(), env);
            TypedExprKind::Binary {
                op: binary.op.clone(),
                lhs,
                rhs,
            }
        }

        Expr::FnCall(fn_call) => {
            let target = resolve_fn_call_target(&fn_call.path.value, scope.tag_types);
            let args = fn_call.args.as_ref().map(|args| {
                args.iter()
                    .map(|a| lower_typed_expr(typed, a, &scope.child(), env))
                    .collect()
            });
            TypedExprKind::FnCall { target, args }
        }

        Expr::TagCall(tag_call) => {
            let variant_id = resolve_tag_call_variant(
                tag_call,
                scope.variant_map,
                scope.tag_types,
                scope.expected_ty,
            );
            let disc = resolve_discriminant(&variant_id, scope.variant_map);
            let field_names = extract_tag_call_field_names(&tag_call.args);
            let args: Vec<ExprId> = tag_call
                .args
                .iter()
                .map(|a| lower_tag_call_arg(typed, a, scope, env))
                .collect();
            TypedExprKind::TagCall {
                variant_id,
                discriminant: disc,
                args: Some(args),
                field_names,
            }
        }

        Expr::AnonymousTag(name) => {
            let variant_id = resolve_tag_call_variant(
                &ast::expr::TagCall {
                    name: *name,
                    qual_path: None,
                    args: vec![],
                },
                scope.variant_map,
                scope.tag_types,
                scope.expected_ty,
            );
            let disc = resolve_discriminant(&variant_id, scope.variant_map);
            TypedExprKind::TagCall {
                variant_id,
                discriminant: disc,
                args: Some(vec![]),
                field_names: Vec::new(),
            }
        }

        Expr::Bind(bind) => {
            // Register constant bindings for the `ReassignConstant` check.
            if bind.is_constant {
                env.constants.insert(bind.name);
            }

            if bind.params.is_some() && bind.params.as_ref().is_some_and(|p| !p.is_empty()) {
                // Function-like bind (inline fn)
                let target = DefId(bind.name);
                let args: Option<Vec<ExprId>> = None;
                TypedExprKind::FnCall { target, args }
            } else if env.locals.contains(&bind.name) {
                // Reassigning an existing variable — produce Reassign.
                let value = match &bind.value {
                    BindValue::Expr(inner) => lower_typed_expr(typed, inner, &scope.child(), env),
                    BindValue::Body { exprs, ret } => {
                        let mut lowered: Vec<ExprId> = exprs
                            .iter()
                            .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                            .collect();
                        ret.value
                            .as_ref()
                            .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                            .or_else(|| lowered.pop())
                            .unwrap_or(ExprId(0))
                    }
                    _ => ExprId(0),
                };
                TypedExprKind::Reassign {
                    name: bind.name,
                    value,
                }
            } else {
                let (stmts, body, has_value) = match &bind.value {
                    BindValue::Expr(inner) => (
                        vec![],
                        lower_typed_expr(typed, inner, &scope.child(), env),
                        true,
                    ),
                    BindValue::Body { exprs, ret } => {
                        let mut lowered: Vec<ExprId> = exprs
                            .iter()
                            .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                            .collect();
                        let body = ret
                            .value
                            .as_ref()
                            .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                            .or_else(|| lowered.pop());
                        match body {
                            Some(body) => (lowered, body, true),
                            None => (vec![], ExprId(0), false),
                        }
                    }
                    BindValue::Extern | BindValue::Unassigned => (vec![], ExprId(0), false),
                };

                let declared_ty = bind_explicit_ty(bind, scope.tag_types);
                if has_value
                    && let Some(explicit) = &declared_ty
                    && body.as_usize() < typed.exprs.ty.len()
                {
                    typed.exprs.ty[body.as_usize()] = explicit.clone();
                    if let Some(values) = explicit.union_literal_values() {
                        annotate_literal_union_literal(typed, body, explicit, values);
                    } else {
                        typed.exprs.const_value[body.as_usize()] = None;
                    }
                }
                let local_ty = declared_ty.unwrap_or_else(|| {
                    typed
                        .exprs
                        .ty
                        .get(body.as_usize())
                        .cloned()
                        .unwrap_or(Ty::Unit)
                });
                env.locals.insert(bind.name);
                env.types.insert(bind.name, local_ty);
                if has_value
                    && let Some(cv) = typed
                        .exprs
                        .const_value
                        .get(body.as_usize())
                        .cloned()
                        .flatten()
                {
                    env.const_values.insert(bind.name, cv);
                }

                TypedExprKind::Bind {
                    name: bind.name,
                    stmts,
                    body,
                    unassigned: !has_value,
                }
            }
        }

        Expr::When(when_expr) => {
            let subject_id = when_expr
                .subject
                .as_ref()
                .map(|s| lower_typed_expr(typed, s, &scope.child(), env));

            let subject_ty = subject_id.and_then(|id| typed.exprs.ty.get(id.as_usize()).cloned());

            let arms = lower_all_when_arms(typed, &when_expr.arms, scope, env, subject_ty.as_ref());

            TypedExprKind::When(TypedWhenExpr {
                subject: subject_id,
                arms,
                body_span: SubSpan::new(SpanId::INVALID),
            })
        }

        Expr::If(if_expr) => {
            let subject_id = lower_typed_expr(typed, &if_expr.subject, &scope.child(), env);
            let stmts: Vec<ExprId> = if_expr
                .body
                .iter()
                .map(|e| lower_typed_expr(typed, e, scope, env))
                .collect();
            let ret = if_expr
                .ret
                .value
                .as_ref()
                .map(|e| lower_typed_expr(typed, e, scope, env));
            let subject_ty = typed.exprs.ty[subject_id.as_usize()].clone();
            let pattern = if_expr
                .pattern
                .clone()
                .or_else(|| happy_pattern_for_subject(typed, &subject_ty, if_expr.subject.span_id));
            let pattern = pattern.unwrap_or_else(|| {
                Box::new(Spanned {
                    value: TypeExpr::Nominal(
                        Intern::from_ref("__MissingHappyVariant"),
                        if_expr.subject.span_id,
                    ),
                    span_id: if_expr.subject.span_id,
                })
            });
            TypedExprKind::If(TypedIfExpr {
                subject: subject_id,
                pattern,
                stmts,
                ret,
                body_span: SubSpan::new(SpanId::INVALID),
            })
        }

        Expr::Loop(loop_enum) => match loop_enum {
            ast::Loop::While(while_loop) => {
                let cond = lower_typed_expr(typed, &while_loop.cond, &scope.child(), env);
                let stmts: Vec<ExprId> = while_loop
                    .exprs
                    .iter()
                    .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                    .collect();
                TypedExprKind::Loop(TypedLoop {
                    kind: TypedLoopKind::While { condition: cond },
                    stmts,
                    keyword_span: SubSpan::new(SpanId::INVALID),
                })
            }
            ast::Loop::ForIn(for_in) => {
                let iter = lower_typed_expr(typed, &for_in.iter, &scope.child(), env);
                let iter_ty = typed.exprs.ty.get(iter.as_usize());
                let elem_ty = iter_ty
                    .and_then(|ty| match ty {
                        Ty::Array { elem, .. } => Some(elem.as_ref().clone()),
                        Ty::Ptr { inner } if inner.is_record() => Some(inner.as_ref().clone()),
                        Ty::Opaque(name) if name.as_str() == "Range" => Some(Ty::Int {
                            width: 64,
                            signed: true,
                            value: None,
                            min: None,
                            max: None,
                        }),
                        _ => None,
                    })
                    .unwrap_or(Ty::i64());

                let _pat_id = lower_typed_expr(typed, &for_in.pat, scope, env);

                let stmts: Vec<ExprId> = for_in
                    .exprs
                    .iter()
                    .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                    .collect();

                let pat_bind_name = match &for_in.pat.value {
                    Expr::Bind(b) => Some(b.name),
                    _ => None,
                };
                if let Some(name) = pat_bind_name {
                    env.types.insert(name, elem_ty);
                }

                let for_loop = TypedLoop {
                    kind: TypedLoopKind::ForIn {
                        variable: pat_bind_name.unwrap_or(Intern::new("_for_var".to_string())),
                        iterable: iter,
                    },
                    stmts,
                    keyword_span: SubSpan::new(SpanId::INVALID),
                };
                TypedExprKind::Loop(for_loop)
            }
        },

        Expr::SelfRef => TypedExprKind::SelfRef {
            target: scope
                .current_def_id
                .unwrap_or(DefId(Intern::new("self".to_string()))),
        },

        Expr::FormatString(fs) => {
            let parts = fs
                .parts
                .iter()
                .map(|p| match p {
                    ast::FormatPart::Text(s) => ast::FormatPart::Text(s.clone()),
                    ast::FormatPart::Expr(e, sp) => ast::FormatPart::Expr(e.clone(), *sp),
                })
                .collect();
            TypedExprKind::FormatString(ast::expr::FormatString { parts })
        }

        Expr::Range(range) => {
            let start = lower_typed_expr(typed, &range.start, &scope.child(), env);
            let end = lower_typed_expr(typed, &range.end, &scope.child(), env);
            TypedExprKind::Range { start, end }
        }

        Expr::TupleAlloc { init, size } => {
            let init_id = lower_typed_expr(typed, init, &scope.child(), env);
            TypedExprKind::TupleAlloc {
                init: init_id,
                size: *size,
            }
        }

        Expr::Destructure {
            field_bindings,
            value,
            ..
        } => {
            let value_id = lower_typed_expr(typed, value, &scope.child(), env);
            TypedExprKind::Destructure {
                value: value_id,
                field_bindings: field_bindings.clone(),
            }
        }

        Expr::RecordSet { base, field, value } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let value_id = lower_typed_expr(typed, value, &scope.child(), env);
            TypedExprKind::RecordSet {
                base: base_id,
                field: *field,
                value: value_id,
            }
        }

        Expr::RecordGet { base, field } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            // Resolve the field index from the base expression's record type.
            let base_ty = &typed.exprs.ty[base_id.as_usize()];
            let field_idx = if let Ty::Record { fields, .. } = base_ty {
                fields
                    .iter()
                    .position(|(n, _)| n.as_str() == field.as_str())
            } else {
                None
            };
            if let Some(idx) = field_idx {
                TypedExprKind::TupleGet {
                    base: base_id,
                    index: idx,
                }
            } else {
                // Fallback: emit a zero literal (same as the wildcard case).
                TypedExprKind::Lit(Literal::Number(0))
            }
        }

        Expr::TupleGet { base, index } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            TypedExprKind::TupleGet {
                base: base_id,
                index: *index,
            }
        }

        Expr::TupleSet { base, index, value } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let value_id = lower_typed_expr(typed, value, &scope.child(), env);
            TypedExprKind::TupleSet {
                base: base_id,
                index: *index,
                value: value_id,
            }
        }

        Expr::BufGet { buf, index } => {
            let base_id = lower_typed_expr(typed, buf, &scope.child(), env);
            let index_id = lower_typed_expr(typed, index, &scope.child(), env);
            TypedExprKind::BufGet {
                buf: base_id,
                index: index_id,
            }
        }

        Expr::BufSet {
            buf, index, value, ..
        } => {
            let base_id = lower_typed_expr(typed, buf, &scope.child(), env);
            let index_id = lower_typed_expr(typed, index, &scope.child(), env);
            let value_id = lower_typed_expr(typed, value, &scope.child(), env);
            TypedExprKind::BufSet {
                buf: base_id,
                index: index_id,
                value: value_id,
            }
        }

        Expr::Cast { expr, ty } => {
            let inner_id = lower_typed_expr(typed, expr, &scope.child(), env);
            let resolved_ty = scope.tag_types.get(ty).cloned().unwrap_or(Ty::Opaque(*ty));
            TypedExprKind::Cast {
                expr: inner_id,
                ty: resolved_ty,
            }
        }

        Expr::TakePtr(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::TakePtr(inner_id)
        }

        Expr::Ref { inner, mutable: _ } => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::Ref(inner_id)
        }

        Expr::Eat(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::Eat(inner_id)
        }

        Expr::ConsumeArg(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::ConsumeArg(inner_id)
        }

        Expr::Deref(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::Deref(inner_id)
        }

        Expr::Negate(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::Negate(inner_id)
        }

        Expr::RecordLit(fields) => {
            let item_ids: Vec<ExprId> = fields
                .iter()
                .map(|(_, e)| lower_typed_expr(typed, e, &scope.child(), env))
                .collect();
            TypedExprKind::TupleLit(item_ids)
        }
        Expr::TupleLit(items) => {
            let item_ids: Vec<ExprId> = items
                .iter()
                .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                .collect();
            TypedExprKind::TupleLit(item_ids)
        }

        Expr::List(items) => {
            let item_ids: Vec<ExprId> = items
                .iter()
                .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                .collect();
            TypedExprKind::List(item_ids)
        }

        Expr::Asm(asm) => {
            let operand_values: Vec<ExprId> = asm
                .operand_values
                .iter()
                .map(|o| lower_typed_expr(typed, o, &scope.child(), env))
                .collect();
            TypedExprKind::Asm(AsmExpr {
                template: asm.template,
                operands: asm.operands.clone(),
                clobbers: asm.clobbers.clone(),
                spec_expr: None,
                operand_values: operand_values
                    .into_iter()
                    .map(|_id| Typed::infer(Expr::Lit(Literal::Number(0)), SpanId::INVALID))
                    .collect(),
            })
        }

        Expr::TypeInRange(_bounds) => TypedExprKind::Bind {
            name: Intern::new("_type_in_range".to_string()),
            stmts: vec![],
            body: ExprId(0),
            unassigned: true,
        },

        Expr::TypeNominal(name) | Expr::TypeGeneric { name, .. } => TypedExprKind::Bind {
            name: *name,
            stmts: vec![],
            body: ExprId(0),
            unassigned: true,
        },

        Expr::TypeQualified(path) => {
            let name = path.segments.last().copied().unwrap_or(path.root);
            TypedExprKind::Bind {
                name,
                stmts: vec![],
                body: ExprId(0),
                unassigned: true,
            }
        }

        Expr::TypeRef { inner, mutable: _ } => {
            let _inner_kind = lower_expr_kind(
                typed,
                &Typed::infer(*inner.clone(), SpanId::INVALID),
                scope,
                env,
            );
            TypedExprKind::Ref(ExprId(0)) // placeholder
        }
    }
}
