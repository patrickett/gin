//! Expression-kind lowering — the main match that converts each [`Expr`]
//! variant to [`TypedExprKind`].
//!
//! This is the central dispatch of the lowering stage. It delegates to
//! sibling modules for tag resolution, type resolution, and when-arm
//! lowering.

use ast::prelude::*;
use ast::span::{SpanId, SubSpan};
use ast::{BinderId, BinderOwner, NormalExpr};
use internment::Intern;

use crate::analysis::unify_type_args_with_registry;
use crate::intrinsic::{ComparisonPredicate, IntegerComparison, IntrinsicOp};
use crate::staging::instantiate_call;
use crate::subst::{DepSubst, DependentInstantiation};
use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{
    BindBody, DefId, ExprId, ReferenceTargetGroup, ReferenceTargetSet, ResolvedCompoundOperator,
    TypedCallableSignature, TypedExprKind, TypedFileAst, TypedIfExpr, TypedLoop, TypedLoopKind,
    TypedWhenExpr,
};
use ast::ty::IntegerInterpretation;

use super::lower_exprs::lower_typed_expr;
use super::lower_exprs::{ExprLowerScope, LocalEnv, join_loop_place_versions, join_place_versions};
use super::lower_tag::{resolve_discriminant, resolve_tag_call_variant};
use super::lower_ty::{
    annotate_literal_union_literal, bind_explicit_ty_in_scope, resolve_fn_call_target,
    resolve_type_reference,
};
use super::lower_when::lower_all_when_arms;

fn root_place_versions_since(
    typed: &TypedFileAst,
    first: usize,
    matches_origin: impl Fn(&crate::typed::PlaceVersionOrigin) -> bool,
) -> Vec<crate::typed::PlaceVersionId> {
    let mut versions = (first..typed.place_versions.len())
        .map(|index| crate::typed::PlaceVersionId(index as u32))
        .filter(|version| {
            let entry = &typed.place_versions[version.0 as usize];
            typed.places[entry.place.0 as usize].parent.is_none() && matches_origin(&entry.origin)
        })
        .collect::<Vec<_>>();
    versions.sort_by_key(|version| typed.place_versions[version.0 as usize].place.0);
    versions
}

fn fallback_tuple_alloc_size(span: SpanId) -> NormalExpr {
    NormalExpr::Var(Intern::new(format!("_unsupported_tuple_size_{span:?}")))
}

fn as_tuple_alloc_size(size: &ast::Expr) -> Option<NormalExpr> {
    match size {
        ast::Expr::Lit(ast::Literal::Int(n)) => Some(NormalExpr::from(*n)),
        ast::Expr::Lit(ast::Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::FnCall(call) if call.args.is_none() => {
            Some(NormalExpr::Var(call.path.value.root))
        }
        ast::Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        ast::Expr::Binary(binary) => {
            let lhs = as_tuple_alloc_size(&binary.lhs.value)?;
            let rhs = as_tuple_alloc_size(&binary.rhs.value)?;
            Some(match binary.op {
                ast::expr::BinOp::Add => NormalExpr::Add(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Subtract => NormalExpr::Sub(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Multiply => NormalExpr::Mul(Box::new(lhs), Box::new(rhs)),
                _ => return None,
            })
        }
        _ => None,
    }
}

fn materialize_integer_operand(typed: &mut TypedFileAst, expression: ExprId, expected: &Ty) {
    let Some(value) = typed
        .exprs
        .const_value_of(expression)
        .and_then(Option::as_ref)
    else {
        return;
    };
    let value = match value {
        ast::ConstValue::Int(value) => *value,
        _ => return,
    };
    if expected
        .named_instance_stripping_reference_wrappers()
        .is_none()
        && expected
            .anonymous_integer_validity()
            .is_some_and(|validity| validity.domain().storage_hull().is_none())
    {
        typed.exprs.ty[expression.index()] = Ty::bounded_int(value, value);
        return;
    }
    if let Err(error) = crate::integer_literal::materialize(typed, expression, expected) {
        let Some(span) = typed.exprs.span_of(expression) else {
            return;
        };
        typed.exprs.flaws[expression.index()].push(
            crate::integer_literal::diagnostic(value, expected, &error)
                .at_span_id(span, &typed.span_table),
        );
    }
}

fn is_numeric_comparison_op(op: &ast::expr::BinOp) -> bool {
    matches!(
        op,
        ast::expr::BinOp::Less
            | ast::expr::BinOp::LessOrEqual
            | ast::expr::BinOp::GreaterOrEqual
            | ast::expr::BinOp::Greater
            | ast::expr::BinOp::Equal
    )
}

fn anonymous_integer_type() -> Ty {
    Ty::i64()
}

fn resolve_compound_operator(
    typed: &mut TypedFileAst,
    scope: &ExprLowerScope,
    operator: &BinOp,
    lhs_ty: &Ty,
    rhs: ExprId,
) -> ResolvedCompoundOperator {
    let role = OperatorRole::for_bin_op(operator).expect("compound operators have binary roles");
    let rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
    match scope.operator_registry.resolve(role, lhs_ty, &rhs_ty) {
        Ok((target, signature)) => ResolvedCompoundOperator::Callable {
            target,
            return_ty: signature.return_type,
        },
        Err(error) => {
            let anonymous =
                |ty: &Ty| ty.is_int() && ty.named_instance_stripping_reference_wrappers().is_none();
            if !anonymous(lhs_ty) || !anonymous(&rhs_ty) {
                return ResolvedCompoundOperator::Invalid(error);
            }
            let interpretation = typed
                .type_registry
                .integer_operation_interpretation_for_type(lhs_ty)
                .or_else(|| {
                    typed
                        .type_registry
                        .integer_operation_interpretation_for_type(&rhs_ty)
                })
                .unwrap_or(IntegerInterpretation::Signed);
            let intrinsic = match role {
                OperatorRole::Add => IntrinsicOp::BitsAdd,
                OperatorRole::Subtract => IntrinsicOp::BitsSubtract,
                OperatorRole::Multiply => IntrinsicOp::BitsMultiply,
                OperatorRole::Divide => match interpretation {
                    IntegerInterpretation::Signed => IntrinsicOp::BitsSignedDivide,
                    IntegerInterpretation::Unsigned => IntrinsicOp::BitsUnsignedDivide,
                },
                OperatorRole::Modulo => match interpretation {
                    IntegerInterpretation::Signed => IntrinsicOp::BitsSignedRemainder,
                    IntegerInterpretation::Unsigned => IntrinsicOp::BitsUnsignedRemainder,
                },
                OperatorRole::BitAnd => IntrinsicOp::BitsAnd,
                OperatorRole::BitOr => IntrinsicOp::BitsOr,
                OperatorRole::BitXor => IntrinsicOp::BitsXor,
                OperatorRole::ShiftLeft => IntrinsicOp::BitsShiftLeft,
                OperatorRole::ShiftRight => match interpretation {
                    IntegerInterpretation::Signed => IntrinsicOp::BitsShiftRightArithmetic,
                    IntegerInterpretation::Unsigned => IntrinsicOp::BitsShiftRightLogical,
                },
                _ => return ResolvedCompoundOperator::Invalid(error),
            };
            ResolvedCompoundOperator::Intrinsic(intrinsic)
        }
    }
}

fn is_numeric_non_comparison_op(op: &ast::expr::BinOp) -> bool {
    matches!(
        op,
        ast::expr::BinOp::Add
            | ast::expr::BinOp::Subtract
            | ast::expr::BinOp::Multiply
            | ast::expr::BinOp::Divide
            | ast::expr::BinOp::Modulo
            | ast::expr::BinOp::BitAnd
            | ast::expr::BinOp::BitOr
            | ast::expr::BinOp::BitXor
            | ast::expr::BinOp::ShiftLeft
            | ast::expr::BinOp::ShiftRight
    )
}

fn infer_return_type_from_expected(
    typed: &TypedFileAst,
    return_type: &Ty,
    expected: Option<&Ty>,
) -> Option<Ty> {
    let expected = expected?;
    let return_instance = return_type.named_instance_stripping_reference_wrappers()?;
    let expected_instance = expected.named_instance_stripping_reference_wrappers()?;
    if return_instance.declaration != expected_instance.declaration
        || return_instance.arguments.len() != expected_instance.arguments.len()
    {
        return None;
    }

    let expected_args: Vec<_> = expected_instance
        .arguments
        .iter()
        .map(|(_, arg)| arg.clone())
        .collect();
    let return_args: Vec<_> = return_instance
        .arguments
        .iter()
        .map(|(_, arg)| arg.clone())
        .collect();
    if let Ok(subst) = unify_type_args_with_registry(
        return_args.as_slice(),
        expected_args.as_slice(),
        Some(&typed.type_registry),
    ) {
        let inferred = subst.apply_to_ty(return_type);
        return Some(inferred);
    }

    let mut inferred_arguments = Vec::with_capacity(return_instance.arguments.len());
    let mut changed = false;
    for (name, return_arg) in &return_instance.arguments {
        let inferred_arg = expected_instance
            .arguments
            .iter()
            .find(|(expected_name, _)| expected_name == name)
            .map_or_else(
                || return_arg.clone(),
                |(_, expected_arg)| match (return_arg, expected_arg) {
                    (ast::TyArg::Type(_), ast::TyArg::Type(_) | ast::TyArg::Const(_))
                    | (ast::TyArg::Const(_), ast::TyArg::Type(_) | ast::TyArg::Const(_)) => {
                        changed = true;
                        expected_arg.clone()
                    }
                },
            );
        changed |= inferred_arg != *return_arg;
        inferred_arguments.push((*name, inferred_arg));
    }
    if !changed {
        return None;
    }

    let mut inferred = return_type.clone();
    let inferred = match &mut inferred {
        Ty::Named { instance, .. } => {
            instance.arguments = inferred_arguments;
            let inferred_instance = instance
                .arguments
                .iter()
                .map(|(name, arg)| (*name, arg.clone()))
                .collect::<Vec<_>>();
            let substitution = DepSubst::from_ty_args(&inferred_instance);
            substitution.apply_to_ty(&inferred)
        }
        _ => return None,
    };

    Some(inferred)
}

fn substitution_from_expected_return(
    typed: &TypedFileAst,
    return_type: &Ty,
    expected: Option<&Ty>,
) -> Option<DepSubst> {
    let expected = expected?;
    let return_instance = return_type.named_instance_stripping_reference_wrappers()?;
    let expected_instance = expected.named_instance_stripping_reference_wrappers()?;
    if return_instance.declaration != expected_instance.declaration
        || return_instance.arguments.len() != expected_instance.arguments.len()
    {
        return None;
    }
    let return_args = return_instance
        .arguments
        .iter()
        .map(|(_, argument)| argument.clone())
        .collect::<Vec<_>>();
    let expected_args = expected_instance
        .arguments
        .iter()
        .map(|(_, argument)| argument.clone())
        .collect::<Vec<_>>();
    unify_type_args_with_registry(&return_args, &expected_args, Some(&typed.type_registry))
        .ok()
        .or_else(|| Some(DepSubst::from_ty_args(&expected_instance.arguments)))
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
        return lower_typed_expr(typed, inner, scope, env);
    }
    lower_typed_expr(typed, arg, scope, env)
}

/// Convert a parse-tree [`Expr`] to a [`TypedExprKind`] by recursively
/// lowering sub-expressions and resolving types.
fn add_destructure_bindings(
    typed: &TypedFileAst,
    value: ExprId,
    tag_name: Intern<String>,
    field_bindings: &[(Intern<String>, Intern<String>)],
    env: &mut LocalEnv,
) {
    let Some(value_ty) = typed.exprs.ty_of(value) else {
        return;
    };
    let value_ty = reference_pointee_for_type(value_ty, Some(&typed.type_registry))
        .unwrap_or(value_ty.clone());
    let value_ty = typed.type_registry.resolved_definition_for_type(&value_ty);
    let value_targets = typed
        .exprs
        .target_group_of(value)
        .and_then(|targets| targets.as_ref());
    let (fields, variant_name, subst) = match &value_ty {
        Ty::Record { fields, .. } => (fields, None, None),
        Ty::Union {
            variants,
            resolved_params,
            ..
        } => {
            let Some(variant) = variants.iter().find(|variant| variant.name == tag_name) else {
                return;
            };
            (
                &variant.fields,
                Some(variant.name),
                resolved_params
                    .as_ref()
                    .map(|args| DepSubst::from_ty_args(args)),
            )
        }
        _ => return,
    };
    let projected_base = variant_name.zip(value_targets).map(|(name, targets)| {
        targets.projected(|target| ReferenceTargetGroup::Variant {
            base: Box::new(target.clone()),
            name,
        })
    });
    for (field_name, binding_name) in field_bindings {
        let Some((index, (_, field_type))) = fields
            .iter()
            .enumerate()
            .find(|(_, (name, _))| name == field_name)
        else {
            continue;
        };
        let field_type = subst
            .as_ref()
            .map(|subst| subst.apply_to_ty(field_type))
            .unwrap_or_else(|| (**field_type).clone());
        env.types.insert(*binding_name, field_type);
        env.locals.insert(*binding_name);
        env.constants.insert(*binding_name);
        let base_targets = projected_base.as_ref().or(value_targets);
        if let Some(base_targets) = base_targets {
            env.target_groups.insert(
                *binding_name,
                base_targets.projected(|target| ReferenceTargetGroup::Field {
                    base: Box::new(target.clone()),
                    index,
                }),
            );
        }
    }
}

fn is_reference_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    typed.type_registry.reference_former_for_type(ty).is_some()
}

pub(crate) fn lower_expr_kind(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
    reserved_expr_id: Option<ExprId>,
) -> TypedExprKind {
    match &expr.value {
        Expr::Lit(lit) => TypedExprKind::Lit(lit.clone()),

        Expr::Binary(binary) => {
            let lhs = lower_typed_expr(typed, &binary.lhs, &scope.without_literal_default(), env);
            let rhs = lower_typed_expr(typed, &binary.rhs, &scope.without_literal_default(), env);
            let mut lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
            let mut rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            if is_numeric_comparison_op(&binary.op) {
                if matches!(rhs_ty, Ty::UnresolvedLiteral(_))
                    && lhs_ty
                        .named_instance_stripping_reference_wrappers()
                        .is_some()
                {
                    materialize_integer_operand(typed, rhs, &lhs_ty);
                    rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
                } else {
                    if matches!(lhs_ty, Ty::UnresolvedLiteral(_)) {
                        materialize_integer_operand(typed, lhs, &anonymous_integer_type());
                        lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
                    }
                    if matches!(rhs_ty, Ty::UnresolvedLiteral(_)) {
                        materialize_integer_operand(typed, rhs, &anonymous_integer_type());
                        rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
                    }
                }
            } else if matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                && !matches!(rhs_ty, Ty::UnresolvedLiteral(_))
            {
                materialize_integer_operand(typed, lhs, &rhs_ty);
                lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
            } else if matches!(rhs_ty, Ty::UnresolvedLiteral(_))
                && !matches!(lhs_ty, Ty::UnresolvedLiteral(_))
            {
                materialize_integer_operand(typed, rhs, &lhs_ty);
                rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            } else if matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                && matches!(rhs_ty, Ty::UnresolvedLiteral(_))
                && scope.integer_literal_defaults.len() == 1
            {
                let expected = &scope.integer_literal_defaults[0];
                materialize_integer_operand(typed, lhs, expected);
                materialize_integer_operand(typed, rhs, expected);
                lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
                rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            } else if is_numeric_non_comparison_op(&binary.op)
                && matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                && matches!(rhs_ty, Ty::UnresolvedLiteral(_))
            {
                materialize_integer_operand(typed, lhs, &anonymous_integer_type());
                materialize_integer_operand(typed, rhs, &anonymous_integer_type());
                lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
                rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            } else if is_numeric_non_comparison_op(&binary.op)
                && lhs_ty.is_int()
                && rhs_ty.is_int()
                && lhs_ty
                    .named_instance_stripping_reference_wrappers()
                    .is_none()
                && rhs_ty
                    .named_instance_stripping_reference_wrappers()
                    .is_none()
            {
                let lhs_width = typed
                    .type_registry
                    .integer_width_for_type(&lhs_ty)
                    .and_then(|width| u8::try_from(width).ok())
                    .unwrap_or(64);
                let rhs_width = typed
                    .type_registry
                    .integer_width_for_type(&rhs_ty)
                    .and_then(|width| u8::try_from(width).ok())
                    .unwrap_or(64);
                let width = lhs_width.max(rhs_width);
                let signed = matches!(
                    typed
                        .type_registry
                        .integer_operation_interpretation_for_type(&lhs_ty)
                        .or_else(|| {
                            typed
                                .type_registry
                                .integer_operation_interpretation_for_type(&rhs_ty)
                        }),
                    Some(IntegerInterpretation::Signed)
                );
                if lhs_width != width || rhs_width != width {
                    let expected = Ty::anonymous_integer_for_width(width, signed);
                    lhs_ty = expected.clone();
                    rhs_ty = expected;
                }
            }
            let Some(role) = OperatorRole::for_bin_op(&binary.op) else {
                return TypedExprKind::Binary {
                    op: binary.op.clone(),
                    lhs,
                    rhs,
                };
            };
            let is_comparison = matches!(
                role,
                OperatorRole::Less
                    | OperatorRole::LessOrEqual
                    | OperatorRole::Greater
                    | OperatorRole::GreaterOrEqual
                    | OperatorRole::Equal
            );
            match scope.operator_registry.resolve(role, &lhs_ty, &rhs_ty) {
                Ok((target, signature)) => TypedExprKind::FnCall {
                    target,
                    args: Some(vec![lhs, rhs]),
                    operator_role: Some(role),
                    substituted_ty: Some(signature.return_type),
                },
                Err(error) => {
                    let lhs_is_anonymous_int = lhs_ty.is_int()
                        && lhs_ty
                            .named_instance_stripping_reference_wrappers()
                            .is_none();
                    let rhs_is_anonymous_int = rhs_ty.is_int()
                        && rhs_ty
                            .named_instance_stripping_reference_wrappers()
                            .is_none();
                    if is_numeric_non_comparison_op(&binary.op)
                        && matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                        && matches!(rhs_ty, Ty::UnresolvedLiteral(_))
                    {
                        let anonymous = anonymous_integer_type();
                        materialize_integer_operand(typed, lhs, &anonymous);
                        materialize_integer_operand(typed, rhs, &anonymous);
                        lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
                    }
                    let anonymous_op = if lhs_is_anonymous_int && rhs_is_anonymous_int {
                        let interpretation = typed
                            .type_registry
                            .integer_operation_interpretation_for_type(&lhs_ty)
                            .or_else(|| {
                                typed
                                    .type_registry
                                    .integer_operation_interpretation_for_type(&rhs_ty)
                            })
                            .unwrap_or(IntegerInterpretation::Signed);
                        match role {
                            OperatorRole::Add => Some(IntrinsicOp::BitsAdd),
                            OperatorRole::Subtract => Some(IntrinsicOp::BitsSubtract),
                            OperatorRole::Multiply => Some(IntrinsicOp::BitsMultiply),
                            OperatorRole::Divide => Some(match interpretation {
                                IntegerInterpretation::Signed => IntrinsicOp::BitsSignedDivide,
                                IntegerInterpretation::Unsigned => IntrinsicOp::BitsUnsignedDivide,
                            }),
                            OperatorRole::Modulo => Some(match interpretation {
                                IntegerInterpretation::Signed => IntrinsicOp::BitsSignedRemainder,
                                IntegerInterpretation::Unsigned => {
                                    IntrinsicOp::BitsUnsignedRemainder
                                }
                            }),
                            OperatorRole::BitAnd => Some(IntrinsicOp::BitsAnd),
                            OperatorRole::BitOr => Some(IntrinsicOp::BitsOr),
                            OperatorRole::BitXor => Some(IntrinsicOp::BitsXor),
                            OperatorRole::ShiftLeft => Some(IntrinsicOp::BitsShiftLeft),
                            OperatorRole::ShiftRight => Some(match interpretation {
                                IntegerInterpretation::Signed => {
                                    IntrinsicOp::BitsShiftRightArithmetic
                                }
                                IntegerInterpretation::Unsigned => {
                                    IntrinsicOp::BitsShiftRightLogical
                                }
                            }),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    if let Some(op) = anonymous_op {
                        return TypedExprKind::IntrinsicCall {
                            op,
                            args: vec![lhs, rhs],
                        };
                    }
                    if is_comparison && lhs_is_anonymous_int && rhs_is_anonymous_int {
                        let interpretation = typed
                            .type_registry
                            .integer_operation_interpretation_for_type(&lhs_ty)
                            .unwrap_or(IntegerInterpretation::Signed);
                        let predicate = match role {
                            OperatorRole::Less => Some(ComparisonPredicate::Less),
                            OperatorRole::LessOrEqual => Some(ComparisonPredicate::LessOrEqual),
                            OperatorRole::Greater => Some(ComparisonPredicate::Greater),
                            OperatorRole::GreaterOrEqual => {
                                Some(ComparisonPredicate::GreaterOrEqual)
                            }
                            OperatorRole::Equal => Some(ComparisonPredicate::Equal),
                            _ => None,
                        };
                        if let Some(predicate) = predicate {
                            let op = IntrinsicOp::BitsCompare {
                                interpretation: match interpretation {
                                    IntegerInterpretation::Signed => IntegerComparison::Signed,
                                    IntegerInterpretation::Unsigned => IntegerComparison::Unsigned,
                                },
                                predicate,
                            };
                            return TypedExprKind::IntrinsicCall {
                                op,
                                args: vec![lhs, rhs],
                            };
                        }
                    }
                    TypedExprKind::InvalidOperator {
                        role,
                        lhs,
                        rhs,
                        error,
                    }
                }
            }
        }

        Expr::FnCall(fn_call) => {
            let mut target = resolve_fn_call_target(&fn_call.path.value, scope.tag_types);
            let parsed_args = fn_call.args.as_deref().unwrap_or_default();
            let self_qualified = !target.0.as_str().contains('.')
                && parsed_args
                    .first()
                    .is_some_and(|argument| match &argument.value {
                        Expr::SelfRef => true,
                        Expr::AnonymousTag(name) => name.as_str() == "Self",
                        Expr::TagCall(call) => call.name.as_str() == "Self" && call.args.is_empty(),
                        _ => false,
                    });
            let call_args = if self_qualified {
                &parsed_args[1..]
            } else {
                parsed_args
            };
            if fn_call.path.value.root.as_str() == "Self"
                && let Some(receiver) = scope.receiver_type.and_then(Ty::type_name)
            {
                let mut parts = vec![receiver.as_str()];
                parts.extend(
                    fn_call
                        .path
                        .value
                        .segments
                        .iter()
                        .map(|segment| segment.as_str()),
                );
                target = DefId(Intern::new(parts.join(".")));
            }
            let arity = call_args.len();
            if !typed.defs.contains_key(&target)
                && !scope.callable_signatures.contains_key(&target)
                && !target.0.as_str().contains('.')
                && let Some(receiver) = scope.receiver_type.and_then(Ty::type_name)
            {
                let receiver_target = DefId(Intern::new(format!("{receiver}.{}", target.0)));
                let receiver_overload =
                    DefId(Intern::new(format!("{}$arity{arity}", receiver_target.0)));
                if typed.defs.contains_key(&receiver_target)
                    || scope.callable_signatures.contains_key(&receiver_target)
                    || typed.defs.contains_key(&receiver_overload)
                    || scope.callable_signatures.contains_key(&receiver_overload)
                {
                    target = receiver_target;
                }
            }
            let overload = DefId(Intern::new(format!("{}$arity{}", target.0, arity)));
            if !typed.defs.contains_key(&target)
                && !scope.callable_signatures.contains_key(&target)
                && (typed.defs.contains_key(&overload)
                    || scope.callable_signatures.contains_key(&overload))
            {
                target = overload;
            }
            if fn_call.path.value.segments.is_empty()
                && let Some(return_type) = env.local_callable_returns.get(&target.0).cloned()
            {
                let args = call_args
                    .iter()
                    .map(|argument| lower_typed_expr(typed, argument, &scope.child(), env))
                    .collect();
                return TypedExprKind::FnCall {
                    target,
                    args: fn_call.args.is_some().then_some(args),
                    operator_role: None,
                    substituted_ty: Some(return_type),
                };
            }
            let local_signature = typed.defs.get(&target).map(TypedCallableSignature::from);
            let signature = local_signature
                .as_ref()
                .or_else(|| scope.callable_signatures.get(&target));
            let args = call_args;
            let lowered_args: Vec<ExprId> = args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    let expected = signature
                        .and_then(|signature| signature.params.get(index))
                        .map(|(_, ty)| ty);
                    lower_typed_expr(typed, arg, &scope.with_expected(expected), env)
                })
                .collect();
            let has_arg_exprs = fn_call.args.is_some();
            if let Some(op) = signature.and_then(|signature| signature.intrinsic) {
                return TypedExprKind::IntrinsicCall {
                    op,
                    args: lowered_args,
                };
            }
            let call_signature = signature.map(|signature| {
                let call_id = reserved_expr_id.expect("function calls reserve an expression ID");
                let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(call_id.0));
                let mut instantiation =
                    DependentInstantiation::new(signature.dependent_binder, binder);
                let mut params: Vec<(Intern<String>, Ty)> = signature
                    .params
                    .iter()
                    .map(|(name, ty)| (*name, instantiation.apply_to_ty(ty)))
                    .collect();
                let mut return_type = instantiation.apply_to_ty(&signature.return_type);
                if let Some(substitution) =
                    substitution_from_expected_return(typed, &return_type, scope.expected_ty)
                {
                    for (_, parameter) in &mut params {
                        *parameter = substitution.apply_to_ty(parameter);
                    }
                    return_type = substitution.apply_to_ty(&return_type);
                }
                (signature.param_kinds.clone(), params, return_type)
            });
            let call_inst = call_signature
                .as_ref()
                .map(|(param_kinds, params, return_type)| {
                    let instantiation =
                        instantiate_call(&lowered_args, typed, param_kinds, params, return_type);
                    (instantiation, return_type)
                });
            let lowered = call_inst
                .as_ref()
                .and_then(|(call_inst, _)| call_inst.args.clone())
                .or_else(|| {
                    if has_arg_exprs {
                        Some(lowered_args.clone())
                    } else {
                        None
                    }
                });
            let substituted_ty = if let Some((_, _, return_type)) = call_signature.as_ref() {
                let by_call = call_inst.and_then(|(inst, _)| inst.substituted_ty);
                let by_expected =
                    infer_return_type_from_expected(typed, return_type, scope.expected_ty);
                by_expected
                    .or(by_call)
                    .or_else(|| Some(return_type.clone()))
            } else {
                None
            };
            TypedExprKind::FnCall {
                target,
                args: lowered,
                operator_role: None,
                substituted_ty,
            }
        }

        Expr::TagCall(tag_call) => {
            let self_call;
            let receiver_definition = scope
                .receiver_type
                .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
            let tag_call = if tag_call.name.as_str() == "Self"
                && let Some(Ty::Record { name, .. } | Ty::Union { name, .. }) =
                    receiver_definition.as_ref()
            {
                self_call = ast::expr::TagCall {
                    name: *name,
                    qual_path: None,
                    args: tag_call.args.clone(),
                };
                &self_call
            } else {
                tag_call
            };
            let variant_id = resolve_tag_call_variant(
                tag_call,
                scope.variant_map,
                scope.tag_types,
                scope.expected_ty,
            );
            let disc = result_alternative_discriminant(scope.expected_ty, variant_id.name)
                .unwrap_or_else(|| resolve_discriminant(&variant_id, scope.variant_map));
            let field_names = extract_tag_call_field_names(&tag_call.args);
            let expected_fields = scope
                .expected_ty
                .and_then(|expected| {
                    match typed.type_registry.resolved_definition_for_type(expected) {
                        Ty::Record { fields, .. } => Some(fields.clone()),
                        Ty::Union { variants, .. } => variants
                            .iter()
                            .find(|variant| variant.name == variant_id.name)
                            .map(|variant| variant.fields.clone()),
                        _ => None,
                    }
                })
                .or_else(|| {
                    typed.tags.get(&variant_id.union).and_then(|tag| {
                        match typed
                            .type_registry
                            .resolved_definition_for_type(&tag.resolved_ty)
                        {
                            Ty::Record { fields, .. } => Some(fields.clone()),
                            Ty::Union { variants, .. } => variants
                                .iter()
                                .find(|variant| variant.name == variant_id.name)
                                .map(|variant| variant.fields.clone()),
                            _ => None,
                        }
                    })
                });
            let args: Vec<ExprId> = tag_call
                .args
                .iter()
                .enumerate()
                .map(|(index, arg)| {
                    let expected = expected_fields.as_ref().and_then(|fields| {
                        let field_name = field_names[index];
                        if field_name.as_str().is_empty() {
                            fields.get(index).map(|(_, ty)| ty.as_ref())
                        } else {
                            fields
                                .iter()
                                .find(|(name, _)| *name == field_name)
                                .map(|(_, ty)| ty.as_ref())
                        }
                    });
                    lower_tag_call_arg(typed, arg, &scope.with_expected(expected), env)
                })
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
            let disc = result_alternative_discriminant(scope.expected_ty, variant_id.name)
                .unwrap_or_else(|| resolve_discriminant(&variant_id, scope.variant_map));
            TypedExprKind::TagCall {
                variant_id,
                discriminant: disc,
                args: Some(vec![]),
                field_names: Vec::new(),
            }
        }

        Expr::Bind(bind) => {
            // Register constant bindings for the `ReassignConstant` check.
            if bind.is_constant() {
                env.constants.insert(bind.name);
            }

            if bind.params.is_some() {
                let bind_id = reserved_expr_id.expect("local callables reserve an expression ID");
                let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(bind_id.0));
                let expected = local_callable_result_type(bind, binder, scope.tag_decls);
                let has_explicit_contract = expected.is_some();
                if let Some(expected) = &expected {
                    env.local_callable_returns
                        .insert(bind.name, expected.clone());
                }
                let mut callable_env = env.clone();
                for name in bind.params.iter().flat_map(|params| params.keys()) {
                    callable_env.types.insert(*name, Ty::Opaque(*name));
                    callable_env.locals.insert(*name);
                }
                let callable_scope = scope.with_expected(expected.as_ref());
                let (stmts, body, has_value) = lower_local_callable_body(
                    typed,
                    &bind.value,
                    &callable_scope,
                    &mut callable_env,
                );
                let return_type = expected
                    .or_else(|| typed.exprs.ty_of(body).cloned())
                    .unwrap_or(Ty::Unit);
                env.local_callable_returns
                    .insert(bind.name, return_type.clone());
                env.types.insert(bind.name, return_type.clone());
                let place = crate::typed::PlaceId(typed.places.len() as u32);
                typed.places.push(crate::typed::TypedPlace {
                    binder,
                    name: bind.name,
                    parent: None,
                    projection: None,
                    mutable: !bind.is_constant(),
                    explicit_contract: has_explicit_contract,
                    ty: return_type,
                });
                env.places.insert(bind.name, place);
                let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
                typed.place_versions.push(crate::typed::TypedPlaceVersion {
                    place,
                    origin: crate::typed::PlaceVersionOrigin::Initializer(body),
                    ty: env.types[&bind.name].clone(),
                    integer_knowledge: typed.exprs.integer_knowledge_of(body).cloned().flatten(),
                });
                env.place_versions.insert(bind.name, version);
                TypedExprKind::Bind {
                    name: bind.name,
                    stmts,
                    body,
                    unassigned: !has_value,
                }
            } else if bind.is_rebind() {
                // Reassigning an existing variable — produce Reassign.
                let value = match &bind.value {
                    BindValue::Expr(inner) => {
                        let expected = env
                            .places
                            .get(&bind.name)
                            .and_then(|place| typed.places.get(place.0 as usize))
                            .filter(|place| place.explicit_contract)
                            .map(|place| place.ty.clone());
                        if let ast::BindOperator::Compound(operator) = &bind.operator {
                            let lhs = Typed::infer(
                                Expr::FnCall(ast::expr::FnCall {
                                    path: Spanned::new(
                                        ast::ModPath::new(bind.name, vec![]),
                                        bind.name_span,
                                    ),
                                    args: None,
                                }),
                                bind.name_span,
                            );
                            let compound = Typed::infer(
                                Expr::Binary(ast::expr::Binary::new(
                                    lhs,
                                    operator.clone(),
                                    inner.as_ref().clone(),
                                )),
                                expr.span_id,
                            );
                            lower_typed_expr(
                                typed,
                                &compound,
                                &scope.with_expected(expected.as_ref()),
                                env,
                            )
                        } else {
                            lower_typed_expr(
                                typed,
                                inner,
                                &scope.with_expected(expected.as_ref()),
                                env,
                            )
                        }
                    }
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
                if let Some(evidence) = typed.exprs.result_evidence_of(value).cloned().flatten() {
                    env.result_evidence.insert(bind.name, evidence);
                } else {
                    env.result_evidence.remove(&bind.name);
                }
                if let Some(evidence) = typed.exprs.projected_result_evidence_of(value)
                    && !evidence.is_empty()
                {
                    env.projected_result_evidence
                        .insert(bind.name, evidence.clone());
                } else {
                    env.projected_result_evidence.remove(&bind.name);
                }
                if let Some(place) = env.places.get(&bind.name).copied() {
                    let predecessor = env.place_versions.get(&bind.name).copied();
                    let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
                    typed.place_versions.push(crate::typed::TypedPlaceVersion {
                        place,
                        origin: crate::typed::PlaceVersionOrigin::Rebind { value, predecessor },
                        ty: typed.places[place.0 as usize].ty.clone(),
                        integer_knowledge: typed
                            .exprs
                            .integer_knowledge_of(value)
                            .cloned()
                            .flatten(),
                    });
                    env.place_versions.insert(bind.name, version);
                }
                TypedExprKind::Reassign {
                    name: bind.name,
                    value,
                    operator: match &bind.operator {
                        ast::BindOperator::Compound(operator) => Some(operator.clone()),
                        _ => None,
                    },
                }
            } else {
                let (stmts, body, has_value) = match &bind.value {
                    BindValue::Expr(inner) => {
                        let expected = bind_explicit_ty_in_scope(
                            bind,
                            scope.tag_types,
                            scope.tag_params,
                            scope.tag_decls,
                        );
                        (
                            vec![],
                            lower_typed_expr(
                                typed,
                                inner,
                                &scope.with_expected(expected.as_ref()),
                                env,
                            ),
                            true,
                        )
                    }
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

                let bind_id = reserved_expr_id.expect("local binds reserve an expression ID");
                let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(bind_id.0));
                let initializer_ty = has_value
                    .then(|| typed.exprs.ty_of(body).cloned())
                    .flatten();
                let declared_ty = super::lower_ty::bind_local_explicit_ty(
                    bind,
                    scope.tag_types,
                    scope.tag_params,
                    scope.tag_decls,
                    binder,
                    initializer_ty.as_ref(),
                );
                let has_explicit_contract = declared_ty.is_some();
                if has_value
                    && let Some(explicit) = &declared_ty
                    && body.as_usize() < typed.exprs.ty.len()
                {
                    typed.exprs.ty[body.as_usize()] = explicit.clone();
                    let literal_values = typed
                        .type_registry
                        .resolved_definition_for_type(explicit)
                        .union_literal_values()
                        .map(<[ast::ConstValue]>::to_vec);
                    if let Some(values) = literal_values {
                        annotate_literal_union_literal(typed, body, explicit, &values);
                    } else if !matches!(
                        typed.exprs.const_value_of(body),
                        Some(Some(ast::ConstValue::Int(_)))
                    ) {
                        typed.exprs.const_value[body.as_usize()] = None;
                    }
                }
                let local_ty = declared_ty
                    .unwrap_or_else(|| typed.exprs.ty_of(body).cloned().unwrap_or(Ty::Unit));
                let is_declared_ref = matches!(
                    bind.return_tag.as_deref().map(|tag| tag.value.clone()),
                    Some(ast::Expr::Ref { .. })
                );
                env.locals.insert(bind.name);
                env.types.insert(bind.name, local_ty.clone());
                let place = crate::typed::PlaceId(typed.places.len() as u32);
                typed.places.push(crate::typed::TypedPlace {
                    binder,
                    name: bind.name,
                    parent: None,
                    projection: None,
                    mutable: !bind.is_constant(),
                    explicit_contract: has_explicit_contract,
                    ty: local_ty.clone(),
                });
                env.places.insert(bind.name, place);
                let version = crate::typed::PlaceVersionId(typed.place_versions.len() as u32);
                typed.place_versions.push(crate::typed::TypedPlaceVersion {
                    place,
                    origin: if has_value {
                        crate::typed::PlaceVersionOrigin::Initializer(body)
                    } else {
                        crate::typed::PlaceVersionOrigin::Declared
                    },
                    ty: local_ty.clone(),
                    integer_knowledge: has_value
                        .then(|| typed.exprs.integer_knowledge_of(body).cloned().flatten())
                        .flatten(),
                });
                env.place_versions.insert(bind.name, version);
                let target_group = if is_declared_ref || is_reference_type(typed, &local_ty) {
                    typed.exprs.target_group_of(body).cloned().flatten()
                } else {
                    Some(ReferenceTargetSet::singleton(ReferenceTargetGroup::Local(
                        place,
                    )))
                };
                if let Some(target_group) = target_group {
                    env.target_groups.insert(bind.name, target_group);
                } else {
                    env.target_groups.remove(&bind.name);
                }
                if has_value
                    && let Some(cv) = typed.exprs.const_value_of(body).and_then(Option::as_ref)
                {
                    env.const_values.insert(bind.name, cv.clone());
                }
                if has_value
                    && let Some(evidence) = typed.exprs.result_evidence_of(body).cloned().flatten()
                {
                    env.result_evidence.insert(bind.name, evidence);
                } else {
                    env.result_evidence.remove(&bind.name);
                }
                if has_value
                    && let Some(evidence) = typed.exprs.projected_result_evidence_of(body)
                    && !evidence.is_empty()
                {
                    env.projected_result_evidence
                        .insert(bind.name, evidence.clone());
                } else {
                    env.projected_result_evidence.remove(&bind.name);
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

            let subject_ty = subject_id.and_then(|id| typed.exprs.ty_of(id).cloned());
            let subject_targets =
                subject_id.and_then(|id| typed.exprs.target_group_of(id).cloned().flatten());

            let first_join = typed.place_versions.len();
            let arms = lower_all_when_arms(
                typed,
                &when_expr.arms,
                scope,
                env,
                subject_ty.as_ref(),
                subject_targets.as_ref(),
            );
            let place_joins = root_place_versions_since(typed, first_join, |origin| {
                matches!(origin, crate::typed::PlaceVersionOrigin::Join(_))
            });

            TypedExprKind::When(TypedWhenExpr {
                subject: subject_id,
                arms,
                place_joins,
                body_span: SubSpan::new(SpanId::INVALID),
            })
        }

        Expr::If(if_expr) => {
            let condition =
                super::lower_when::lower_condition(typed, &if_expr.condition, scope, env);
            let incoming = env.clone();
            let mut branch_env = incoming.clone();
            refine_integer_locals_from_condition(typed, &condition, true, &mut branch_env);
            let branch_scope = scope.temporary_child();
            let mut exit_scope = scope.with_expected(scope.expected_ty);
            exit_scope.temporary_shadowing = true;
            let last_statement = if_expr.body.len().saturating_sub(1);
            let stmts: Vec<ExprId> = if_expr
                .body
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let expr_scope = if index == last_statement {
                        &exit_scope
                    } else {
                        &branch_scope
                    };
                    lower_typed_expr(typed, e, expr_scope, &mut branch_env)
                })
                .collect();
            let ret = if_expr
                .ret
                .value
                .as_ref()
                .map(|e| lower_typed_expr(typed, e, &exit_scope, &mut branch_env));
            let first_join = typed.place_versions.len();
            join_place_versions(typed, env, &incoming, &[branch_env], true);
            let place_joins = root_place_versions_since(typed, first_join, |origin| {
                matches!(origin, crate::typed::PlaceVersionOrigin::Join(_))
            });
            TypedExprKind::If(TypedIfExpr {
                condition,
                stmts,
                ret,
                place_joins,
                body_span: SubSpan::new(SpanId::INVALID),
            })
        }

        Expr::Loop(loop_enum) => match loop_enum {
            ast::Loop::While(while_loop) => {
                let condition =
                    super::lower_when::lower_condition(typed, &while_loop.condition, scope, env);
                let incoming = env.clone();
                let mut body_env = incoming.clone();
                let stmts: Vec<ExprId> = while_loop
                    .exprs
                    .iter()
                    .map(|e| lower_typed_expr(typed, e, &scope.temporary_child(), &mut body_env))
                    .collect();
                let first_phi = typed.place_versions.len();
                join_loop_place_versions(typed, env, &incoming, &body_env);
                let place_phis = root_place_versions_since(typed, first_phi, |origin| {
                    matches!(origin, crate::typed::PlaceVersionOrigin::LoopPhi { .. })
                });
                TypedExprKind::Loop(TypedLoop {
                    kind: TypedLoopKind::While { condition },
                    stmts,
                    place_phis,
                    keyword_span: SubSpan::new(SpanId::INVALID),
                })
            }
            ast::Loop::ForIn(for_in) => {
                let iter = lower_typed_expr(typed, &for_in.iter, &scope.child(), env);
                let iter_ty = typed.exprs.ty_of(iter);
                let elem_ty = iter_ty
                    .and_then(|ty| match ty {
                        Ty::Array { elem, .. } => Some(elem.as_ref().clone()),
                        ty if ty.pointee_ty().is_some_and(Ty::is_record) => {
                            ty.pointee_ty().cloned()
                        }
                        Ty::Opaque(name) if name.as_str() == "Range" => Some(Ty::i64()),
                        _ => None,
                    })
                    .unwrap_or(Ty::i64());

                let incoming = env.clone();
                let mut body_env = incoming.clone();
                let _pat_id = lower_typed_expr(typed, &for_in.pat, scope, &mut body_env);

                let stmts: Vec<ExprId> = for_in
                    .exprs
                    .iter()
                    .map(|e| lower_typed_expr(typed, e, &scope.temporary_child(), &mut body_env))
                    .collect();

                let pat_bind_name = match &for_in.pat.value {
                    Expr::Bind(b) => Some(b.name),
                    _ => None,
                };
                if let Some(name) = pat_bind_name {
                    body_env.types.insert(name, elem_ty);
                }
                let first_phi = typed.place_versions.len();
                join_loop_place_versions(typed, env, &incoming, &body_env);
                let place_phis = root_place_versions_since(typed, first_phi, |origin| {
                    matches!(origin, crate::typed::PlaceVersionOrigin::LoopPhi { .. })
                });

                let for_loop = TypedLoop {
                    kind: TypedLoopKind::ForIn {
                        variable: pat_bind_name.unwrap_or(Intern::new("_for_var".to_string())),
                        iterable: iter,
                    },
                    stmts,
                    place_phis,
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
            let init_expected = (|| {
                if let Some(expected) = scope.expected_ty.map(Ty::without_reference_wrappers)
                    && let Some(value) = match expected {
                        Ty::Named { instance, .. } => {
                            let declaration =
                                typed.type_registry.declaration_for_instance(instance)?;
                            let former = declaration.fixed_array.as_ref()?;
                            instance
                                .arguments
                                .iter()
                                .find_map(|(parameter, arg)| {
                                    (*parameter == former.element_parameter).then(|| match arg {
                                        ast::TyArg::Type(ty) => Some(ty.as_ref().clone()),
                                        _ => None,
                                    })
                                })
                                .and_then(|ty| ty)
                        }
                        Ty::Array { elem, .. } => Some(elem.as_ref().clone()),
                        _ => None,
                    }
                {
                    return Some(value);
                }

                match init.value {
                    ast::Expr::Lit(ast::Literal::Int(_) | ast::Literal::Number(_))
                        if scope.integer_literal_defaults.len() == 1 =>
                    {
                        Some(scope.integer_literal_defaults[0].clone())
                    }
                    _ => None,
                }
            })();
            let init_scope = init_expected
                .as_ref()
                .map(|ty| scope.with_expected(Some(ty)))
                .unwrap_or_else(|| scope.without_literal_default());
            let init_id = lower_typed_expr(typed, init, &init_scope, env);
            let size = as_tuple_alloc_size(&size.value)
                .or_else(|| {
                    let size_expr_id = lower_typed_expr(typed, size, &scope.child(), env);
                    crate::staging::normalize(typed, size_expr_id, &DepSubst::new())
                        .as_dependent_normal_expr()
                })
                .unwrap_or_else(|| fallback_tuple_alloc_size(size.span_id));
            TypedExprKind::TupleAlloc {
                init: init_id,
                size,
            }
        }

        Expr::Destructure {
            tag_name,
            field_bindings,
            value,
        } => {
            let value_id = lower_typed_expr(typed, value, &scope.child(), env);
            add_destructure_bindings(typed, value_id, *tag_name, field_bindings, env);
            TypedExprKind::Destructure {
                tag_name: *tag_name,
                value: value_id,
                field_bindings: field_bindings.clone(),
            }
        }

        Expr::RecordSet {
            base,
            field,
            value,
            operator,
        } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let base_definition = typed.type_registry.resolved_definition_for_type(
                &typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit),
            );
            let expected = match base_definition {
                Ty::Record { fields, .. } => fields
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, ty)| ty.as_ref().clone()),
                _ => None,
            };
            let value_id =
                lower_typed_expr(typed, value, &scope.with_expected(expected.as_ref()), env);
            TypedExprKind::RecordSet {
                base: base_id,
                field: *field,
                value: value_id,
                operator: operator.as_ref().map(|operator| {
                    resolve_compound_operator(
                        typed,
                        scope,
                        operator,
                        expected.as_ref().unwrap_or(&Ty::Unit),
                        value_id,
                    )
                }),
            }
        }

        Expr::RecordGet { base, field } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            // Resolve the field index from the base expression's record type.
            let base_ty = typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit);
            let base_ty = reference_pointee_for_type(&base_ty, Some(&typed.type_registry))
                .unwrap_or(base_ty.clone());
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let field_idx = if let Ty::Record { fields, .. } = &base_definition {
                fields.iter().position(|(name, _)| name == field)
            } else {
                let target = match typed.exprs.kind_of(base_id) {
                    Some(TypedExprKind::FnCall { target, .. }) => Some(*target),
                    _ => None,
                };
                target.and_then(|target| {
                    let bind = typed.defs.get(&target)?;
                    let body = match &bind.body {
                        BindBody::Expr(expr) => Some(*expr),
                        BindBody::Body { exprs, ret } => (*ret).or_else(|| exprs.last().copied()),
                        BindBody::Extern => None,
                    }?;
                    let tag_id = match typed.exprs.kind_of(body) {
                        Some(TypedExprKind::TagCall { variant_id, .. }) => Some(variant_id.union),
                        _ => None,
                    }?;
                    let tag = typed.tags.get(&tag_id)?;
                    let Ty::Record { fields, .. } = typed
                        .type_registry
                        .resolved_definition_for_type(&tag.resolved_ty)
                    else {
                        return None;
                    };
                    fields.iter().position(|(name, _)| name == field)
                })
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

        Expr::TupleSet {
            base,
            index,
            value,
            operator,
        } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let base_definition = typed.type_registry.resolved_definition_for_type(
                &typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit),
            );
            let expected = match base_definition {
                Ty::Tuple(fields) => fields.get(*index).cloned(),
                _ => None,
            };
            let value_id =
                lower_typed_expr(typed, value, &scope.with_expected(expected.as_ref()), env);
            TypedExprKind::TupleSet {
                base: base_id,
                index: *index,
                value: value_id,
                operator: operator.as_ref().map(|operator| {
                    resolve_compound_operator(
                        typed,
                        scope,
                        operator,
                        expected.as_ref().unwrap_or(&Ty::Unit),
                        value_id,
                    )
                }),
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
            buf,
            index,
            value,
            operator,
        } => {
            let base_id = lower_typed_expr(typed, buf, &scope.child(), env);
            let index_id = lower_typed_expr(typed, index, &scope.child(), env);
            let expected = typed.exprs.ty_of(base_id).and_then(|base_ty| {
                base_ty
                    .named_instance_stripping_reference_wrappers()
                    .and_then(|instance| {
                        let declaration = typed.type_registry.declaration_for_instance(instance)?;
                        let fixed_array = declaration.fixed_array.as_ref()?;
                        instance.arguments.iter().find_map(|(parameter, argument)| {
                            (*parameter == fixed_array.element_parameter).then(
                                || match argument {
                                    ast::TyArg::Type(ty) => Some(ty.as_ref().clone()),
                                    _ => None,
                                },
                            )?
                        })
                    })
                    .or_else(|| base_ty.pointee_ty().cloned())
            });
            let value_id =
                lower_typed_expr(typed, value, &scope.with_expected(expected.as_ref()), env);
            TypedExprKind::BufSet {
                buf: base_id,
                index: index_id,
                value: value_id,
                operator: operator.as_ref().map(|operator| {
                    resolve_compound_operator(
                        typed,
                        scope,
                        operator,
                        expected.as_ref().unwrap_or(&Ty::Unit),
                        value_id,
                    )
                }),
            }
        }

        Expr::Cast { expr, ty } => {
            let resolved_ty =
                resolve_type_reference(ty, scope.tag_types, &env.types, Some(scope.tag_params));
            let inner_id =
                lower_typed_expr(typed, expr, &scope.with_anonymous_literal_default(), env);
            TypedExprKind::Cast {
                expr: inner_id,
                ty: resolved_ty,
            }
        }

        Expr::TargetQuery { kind, operand } => TypedExprKind::TargetQuery {
            kind: *kind,
            operand: resolve_type_reference(
                operand,
                scope.tag_types,
                &env.types,
                Some(scope.tag_params),
            ),
        },

        Expr::TakePtr(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::TakePtr(inner_id)
        }

        Expr::Ref { inner, .. } => {
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
            if let Expr::Lit(literal @ (Literal::Int(_) | Literal::Number(_))) = &inner.value {
                TypedExprKind::Lit(literal.clone())
            } else {
                let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
                TypedExprKind::Negate(inner_id)
            }
        }

        Expr::RecordLit(fields) => {
            let expected_definition = scope
                .expected_ty
                .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
            let expected_fields = match &expected_definition {
                Some(Ty::Record { fields, .. }) => Some(fields),
                _ => None,
            };
            let item_ids: Vec<ExprId> = fields
                .iter()
                .map(|(name, e)| {
                    let expected = expected_fields.and_then(|expected_fields| {
                        expected_fields
                            .iter()
                            .find(|(expected_name, _)| expected_name == name)
                            .map(|(_, ty)| ty.as_ref())
                    });
                    lower_typed_expr(typed, e, &scope.with_expected(expected), env)
                })
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
    }
}

fn refine_integer_locals_from_condition(
    typed: &TypedFileAst,
    condition: &crate::typed::TypedCondition,
    taken: bool,
    env: &mut LocalEnv,
) {
    match condition {
        crate::typed::TypedCondition::And(left, right) if taken => {
            refine_integer_locals_from_condition(typed, left, true, env);
            refine_integer_locals_from_condition(typed, right, true, env);
        }
        crate::typed::TypedCondition::Or(left, right) if !taken => {
            refine_integer_locals_from_condition(typed, left, false, env);
            refine_integer_locals_from_condition(typed, right, false, env);
        }
        crate::typed::TypedCondition::Not(inner) => {
            refine_integer_locals_from_condition(typed, inner, !taken, env);
        }
        crate::typed::TypedCondition::Is { subject, pattern } => {
            let ast::Pattern::Nominal(selected, _) = &pattern.value else {
                return;
            };
            let selected = if taken {
                *selected
            } else if selected.as_str() == "True" {
                Intern::from_ref("False")
            } else if selected.as_str() == "False" {
                Intern::from_ref("True")
            } else {
                return;
            };
            let Some(TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            }) = typed.exprs.kind_of(*subject)
            else {
                return;
            };
            let Some(bind) = typed.defs.get(target) else {
                return;
            };
            let Ty::ResultFamily { alternatives, .. } = &bind.return_type else {
                return;
            };
            let Some(proposition) = alternatives
                .iter()
                .find(|alternative| alternative.label == selected)
                .and_then(|alternative| alternative.proposition.as_ref())
            else {
                return;
            };
            for ((parameter, _), argument) in bind.params.iter().zip(args) {
                let Some(local) = local_name_for_expr(typed, *argument) else {
                    continue;
                };
                let predicate = predicate_for_proposition_subject(proposition, *parameter);
                let domain =
                    ast::integer::IntegerDomain::from_named_refinement(&predicate, *parameter);
                if domain.storage_hull().is_some() {
                    env.types.insert(
                        local,
                        Ty::AnonymousInteger {
                            validity: ast::integer::IntegerValidity::new(domain),
                        },
                    );
                }
            }
        }
        crate::typed::TypedCondition::And(_, _) | crate::typed::TypedCondition::Or(_, _) => {}
    }
}

fn local_name_for_expr(typed: &TypedFileAst, expr: ExprId) -> Option<Intern<String>> {
    match typed.exprs.kind_of(expr)? {
        TypedExprKind::FnCall { target, args, .. } if args.is_none() => Some(target.0),
        TypedExprKind::Ref(inner)
        | TypedExprKind::Deref(inner)
        | TypedExprKind::TakePtr(inner)
        | TypedExprKind::ConsumeArg(inner)
        | TypedExprKind::Rematerialize { source: inner, .. } => local_name_for_expr(typed, *inner),
        _ => None,
    }
}

fn predicate_for_proposition_subject(
    proposition: &ast::ProofProposition,
    subject: Intern<String>,
) -> ast::PredicateExpr {
    use ast::{ProofProposition, ProofRelation};
    match proposition {
        ProofProposition::And(left, right) => ast::PredicateExpr::And(vec![
            predicate_for_proposition_subject(left, subject),
            predicate_for_proposition_subject(right, subject),
        ]),
        ProofProposition::Compare {
            left: ast::ProofTerm::Name(name),
            relation,
            right,
        } if *name == subject => {
            let Some(right) = proof_term_as_normal(right) else {
                return ast::PredicateExpr::Proposition(Box::new(proposition.clone()));
            };
            match relation {
                ProofRelation::Equal => ast::PredicateExpr::Eq(right),
                ProofRelation::NotEqual => ast::PredicateExpr::Ne(right),
                ProofRelation::Less => ast::PredicateExpr::Lt(right),
                ProofRelation::LessOrEqual => ast::PredicateExpr::Le(right),
                ProofRelation::Greater => ast::PredicateExpr::Gt(right),
                ProofRelation::GreaterOrEqual => ast::PredicateExpr::Ge(right),
            }
        }
        _ => ast::PredicateExpr::Proposition(Box::new(proposition.clone())),
    }
}

fn proof_term_as_normal(term: &ast::ProofTerm) -> Option<ast::NormalExpr> {
    match term {
        ast::ProofTerm::Value(value) => Some(ast::NormalExpr::Value(ast::ConstValue::Int(*value))),
        ast::ProofTerm::Name(name) => Some(ast::NormalExpr::Var(*name)),
        ast::ProofTerm::Add(left, right) => Some(ast::NormalExpr::Add(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::Sub(left, right) => Some(ast::NormalExpr::Sub(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::Mul(left, right) => Some(ast::NormalExpr::Mul(
            Box::new(proof_term_as_normal(left)?),
            Box::new(proof_term_as_normal(right)?),
        )),
        ast::ProofTerm::TargetQuery { kind, operand } => Some(ast::NormalExpr::TargetQuery {
            kind: *kind,
            operand: operand.clone(),
        }),
        ast::ProofTerm::Remainder(_, _) | ast::ProofTerm::PowerOfTwo(_) => None,
    }
}

fn result_alternative_discriminant(expected: Option<&Ty>, label: Intern<String>) -> Option<usize> {
    let Ty::ResultFamily { alternatives, .. } = expected? else {
        return None;
    };
    alternatives
        .iter()
        .position(|alternative| alternative.label == label)
}

fn local_callable_result_type(
    bind: &Bind,
    binder: BinderId,
    tag_decls: &ast::TagMap,
) -> Option<Ty> {
    let alternatives: Vec<ast::ResultAlternative> = if bind.anonymous_result_alternatives.is_empty()
        && bind.return_tag.is_none()
        && bind.return_type_name.is_none()
        && bind.type_annotation.is_none()
    {
        let mut labels = Vec::new();
        collect_local_result_alternatives_from_value(&bind.value, tag_decls, &mut labels);
        labels
            .into_iter()
            .map(|label| ast::ResultAlternative {
                label,
                proposition: None,
            })
            .collect()
    } else if !bind.anonymous_result_alternatives.is_empty() {
        bind.anonymous_result_alternatives
            .iter()
            .map(|alternative| alternative.value.clone())
            .collect()
    } else {
        Vec::new()
    };
    (!alternatives.is_empty()).then_some(Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::LocalCallable(binder),
        alternatives,
    })
}

fn collect_local_result_alternatives_from_value(
    value: &BindValue,
    tag_decls: &ast::TagMap,
    labels: &mut Vec<Intern<String>>,
) {
    match value {
        BindValue::Expr(expr) => collect_local_result_alternatives(&expr.value, tag_decls, labels),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                if matches!(expr.value, Expr::If(_) | Expr::When(_)) {
                    collect_local_result_alternatives(&expr.value, tag_decls, labels);
                }
            }
            if let Some(exit) = &ret.value {
                collect_local_result_alternatives(&exit.value, tag_decls, labels);
            }
        }
        BindValue::Extern | BindValue::Unassigned => {}
    }
}

fn collect_local_result_alternatives(
    expr: &Expr,
    tag_decls: &ast::TagMap,
    labels: &mut Vec<Intern<String>>,
) {
    let label = match expr {
        Expr::AnonymousTag(label) => Some(*label),
        Expr::TagCall(call) if call.qual_path.is_none() && !tag_decls.contains_key(&call.name) => {
            Some(call.name)
        }
        _ => None,
    };
    if let Some(label) = label {
        if !labels.contains(&label) {
            labels.push(label);
        }
        return;
    }
    match expr {
        Expr::If(if_expr) => {
            if let Some(exit) = if_expr.body.last() {
                collect_local_result_alternatives(&exit.value, tag_decls, labels);
            }
            if let Some(exit) = &if_expr.ret.value {
                collect_local_result_alternatives(&exit.value, tag_decls, labels);
            }
        }
        Expr::When(when_expr) => {
            for arm in &when_expr.arms {
                let exit = match arm {
                    ast::WhenArm::Cond { body, .. } | ast::WhenArm::Is { body, .. } => body,
                    ast::WhenArm::Else(body, _) => body,
                };
                collect_local_result_alternatives(&exit.value, tag_decls, labels);
            }
        }
        _ => {}
    }
}

fn lower_local_callable_body(
    typed: &mut TypedFileAst,
    value: &BindValue,
    scope: &ExprLowerScope<'_>,
    env: &mut LocalEnv,
) -> (Vec<ExprId>, ExprId, bool) {
    match value {
        BindValue::Expr(expr) => (Vec::new(), lower_typed_expr(typed, expr, scope, env), true),
        BindValue::Body { exprs, ret } => {
            let mut lowered = Vec::new();
            for expr in exprs {
                let child = scope.child();
                let expr_scope = if matches!(expr.value, Expr::If(_) | Expr::When(_)) {
                    scope
                } else {
                    &child
                };
                lowered.push(lower_typed_expr(typed, expr, expr_scope, env));
            }
            let body = ret
                .value
                .as_ref()
                .map(|exit| lower_typed_expr(typed, exit, scope, env))
                .or_else(|| lowered.pop());
            match body {
                Some(body) => (lowered, body, true),
                None => (Vec::new(), ExprId(0), false),
            }
        }
        BindValue::Extern | BindValue::Unassigned => (Vec::new(), ExprId(0), false),
    }
}
