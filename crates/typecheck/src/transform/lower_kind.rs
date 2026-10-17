//! Expression-kind lowering — the main match that converts each [`Expr`]
//! variant to [`TypedExprKind`].
//!
//! This is the central dispatch of the lowering stage. It delegates to
//! sibling modules for tag resolution, type resolution, and when-arm
//! lowering.

use crate::tag_declarations::TagDeclarations;
use ast::prelude::*;
use ast::span::{SpanId, SubSpan};
use ast::{BinderId, BinderOwner, ConstValue, NormalExpr};
use diagnostic::code::{
    CallCode, ImportCode, MemberCode, ResultCode, SequenceCode, TextCode, TypeCode,
};
use diagnostic::{Diagnostic, DiagnosticCode};
use internment::Intern;

use crate::analysis::{TypeEnv, unify_type_args_with_registry};
use crate::intrinsic::{ComparisonPredicate, IntegerComparison, IntrinsicOp};
use crate::normal_expr::{SizeCallName, size_expr_to_normal};
use crate::staging::instantiate_call;
use crate::subst::{DepSubst, DependentInstantiation};
use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{
    BindBody, DefId, ExprId, ReferenceTargetGroup, ReferenceTargetSet, ResolvedCompoundOperator,
    TypedCallableSignature, TypedExprKind, TypedFileAst, TypedIfExpr, TypedLoop, TypedLoopKind,
    TypedWhenExpr,
};
use ast::ty::IntegerInterpretation;

use super::collect_result_exit_alternatives;
use super::lower_exprs::{ExprLowerScope, FlowState, LocalCallable, join_loop_place_versions};
use super::lower_exprs::{
    contextualize_local_literal, lower_typed_expr, sequence_element_argument,
};
use super::lower_tag::{resolve_discriminant, resolve_tag_call_variant, type_sum_member};
use super::lower_ty::{
    ExprTypeResolverCtx, annotate_literal_union_literal, bind_explicit_ty_in_scope,
    resolve_expr_type, resolve_fn_call_target, resolve_type_reference,
};
use super::lower_when::lower_all_when_arms;

pub(super) fn root_place_versions_since(
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

fn fallback_fixed_array_alloc_size(span: SpanId) -> NormalExpr {
    NormalExpr::Var(Intern::new(format!(
        "_unsupported_fixed_array_size_{span:?}"
    )))
}

fn lower_array_length(
    typed: &mut TypedFileAst,
    length: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> NormalExpr {
    let id = lower_typed_expr(typed, length, &scope.child(), env);
    crate::staging::normalize(typed, id, &DepSubst::new())
        .as_dependent_normal_expr()
        .or_else(|| size_expr_to_normal(&length.value, SizeCallName::Mangled, false))
        .unwrap_or_else(|| fallback_fixed_array_alloc_size(length.span_id))
}

fn read_anonymous_integer_operand(typed: &mut TypedFileAst, expression: ExprId) -> ExprId {
    if matches!(typed.exprs.ty_of(expression), Some(Ty::Ref { inner, .. }) if matches!(inner.as_ref(), Ty::AnonymousInteger { .. }))
    {
        read_integer_operand(typed, expression)
    } else {
        expression
    }
}

fn read_integer_operand(typed: &mut TypedFileAst, expression: ExprId) -> ExprId {
    let Some(Ty::Ref { inner, .. }) = typed.exprs.ty_of(expression) else {
        return expression;
    };
    if typed
        .type_registry
        .integer_validity_for_type(inner)
        .is_none()
    {
        return expression;
    }
    let ty = *inner.clone();
    super::lower_exprs::append_derived_expr(typed, expression, TypedExprKind::Deref(expression), ty)
}

fn materialize_integer_operand(
    typed: &mut TypedFileAst,
    expression: ExprId,
    expected: Option<&Ty>,
) {
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
    let inferred = Ty::bounded_int(value, value);
    let expected = expected.unwrap_or(&inferred);
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
        typed.exprs.facts.semantic_facts.flaws[expression.index()].push(
            crate::integer_literal::diagnostic(value, expected, &error)
                .at_span_id(span, &typed.span_table),
        );
    }
}

pub(super) fn lower_numeric_operands(
    typed: &mut TypedFileAst,
    left: &Typed<Expr>,
    right: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> (ExprId, ExprId) {
    let scope = scope.without_literal_materialization();
    let left = lower_typed_expr(typed, left, &scope, env);
    let right = lower_typed_expr(typed, right, &scope, env);
    let left = read_integer_operand(typed, left);
    let right = read_integer_operand(typed, right);
    for operand in [left, right] {
        if matches!(
            typed.exprs.ty_of(operand),
            Some(Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer))
        ) {
            materialize_integer_operand(typed, operand, None);
        }
    }
    (left, right)
}

fn anonymous_integer_type() -> Ty {
    Ty::i64()
}

fn fixed_array_element_type(typed: &TypedFileAst, expected: &Ty) -> Option<Ty> {
    match Ty::without_reference_wrappers(expected) {
        Ty::Named { instance, .. } => {
            let declaration = typed.type_registry.declaration_for_instance(instance)?;
            let former = declaration.sequence_literal.as_ref()?;
            let (_, argument) = sequence_element_argument(instance, former)?;
            match argument {
                ast::TyArg::Type(element) => Some(element.as_ref().clone()),
                ast::TyArg::Const(_) => None,
            }
        }
        Ty::Array { elem, .. } => Some(elem.as_ref().clone()),
        _ => None,
    }
}

fn integer_binary_interpretation(
    typed: &TypedFileAst,
    role: OperatorRole,
    operands: [&Ty; 2],
) -> IntegerInterpretation {
    let interpretations = operands.map(|ty| {
        typed
            .type_registry
            .integer_operation_interpretation_for_type(ty)
    });
    if matches!(role, OperatorRole::Divide | OperatorRole::Modulo)
        && interpretations.contains(&Some(IntegerInterpretation::Signed))
    {
        IntegerInterpretation::Signed
    } else {
        interpretations[0]
            .or(interpretations[1])
            .unwrap_or(IntegerInterpretation::Signed)
    }
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
    match scope
        .operator_registry
        .resolve(operator.symbol(), lhs_ty, &rhs_ty)
    {
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
            let interpretation = integer_binary_interpretation(typed, role, [lhs_ty, &rhs_ty]);
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
    let substitution = substitution_from_expected_return(typed, return_type, expected)?;
    Some(substitution.apply_to_ty(return_type))
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
        || !return_instance
            .arguments
            .iter()
            .map(|(name, _)| name)
            .eq(expected_instance.arguments.iter().map(|(name, _)| name))
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
    unify_type_args_with_registry(&return_args, &expected_args, Some(&typed.type_registry)).ok()
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

/// Named argument carriers are not local declarations.
pub(crate) fn lower_call_arg(
    typed: &mut TypedFileAst,
    arg: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> ExprId {
    let arg = if let Expr::Bind(b) = &arg.value
        && b.params.is_none()
        && let BindValue::Expr(inner) = &b.value
    {
        inner
    } else {
        arg
    };
    let existing_reference = match &arg.value {
        Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
            env.types.get(&call.path.root)
        }
        Expr::SelfRef => env.types.get(&Intern::from_ref("self")),
        _ => None,
    }
    .cloned();
    let expected = scope
        .expected_ty
        .map(|ty| match (ty, existing_reference.as_ref()) {
            (
                Ty::Ref { inner, .. },
                Some(
                    actual @ Ty::Ref {
                        inner: referent, ..
                    },
                ),
            ) if inner == referent => actual,
            (Ty::Ref { inner, .. }, _) => inner.as_ref(),
            (ty, _) => ty,
        });
    let child = scope.child();
    let argument_scope = child.with_expected(expected);
    lower_typed_expr(typed, arg, &argument_scope, env)
}

fn lower_named_type_argument(
    typed: &mut TypedFileAst,
    argument: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
) -> Option<ExprId> {
    let argument = match &argument.value {
        Expr::Bind(bind) => match &bind.value {
            BindValue::Expr(value) => value,
            _ => return None,
        },
        _ => argument,
    };
    let name = match &argument.value {
        Expr::AnonymousTag(name) => *name,
        Expr::TagCall(call) => call.name,
        _ => return None,
    };
    if !scope.tag_types.contains_key(&name) {
        return None;
    }
    let ty = TypeEnv::new(scope.tag_types)
        .with_tag_params(scope.tag_params)
        .with_tag_declaration_lookup(scope.tag_decls)
        .with_registry(&typed.type_registry)
        .resolve_expr(&argument.value);
    let id = ExprId(typed.exprs.kind.len() as u32);
    typed.exprs.push(crate::typed::TypedExpr {
        kind: TypedExprKind::TagCall {
            variant_id: crate::typed::VariantId {
                union: crate::TagId(name),
                name,
            },
            discriminant: 0,
            args: None,
            field_names: Vec::new(),
        },
        ty,
        span: argument.span_id,
        facts: crate::typed::ExprFacts {
            const_facts: crate::typed::ConstFacts {
                const_value: None,
                const_object: None,
                sequence_literal: None,
                literal_materialization: None,
            },
            semantic_facts: crate::typed::SemanticFacts {
                integer_knowledge: None,
                result_evidence: None,
                projected_result_evidence: Default::default(),
                availability: crate::typed::Availability::CompileTime,
                flaws: Vec::new(),
            },
            ownership_facts: crate::typed::OwnershipFacts {
                references: Default::default(),
                reference_reads: Default::default(),
                target_group: None,
                result_target_groups: Vec::new(),
                place: None,
                place_version: None,
            },
        },
    });
    Some(id)
}

/// Convert a parse-tree [`Expr`] to a [`TypedExprKind`] by recursively
/// lowering sub-expressions and resolving types.
fn add_destructure_bindings(
    typed: &mut TypedFileAst,
    value: ExprId,
    tag_name: Intern<String>,
    field_bindings: &[(Intern<String>, Intern<String>)],
    env: &mut FlowState,
) -> Vec<crate::typed::PlaceId> {
    let Some(value_ty) = typed.exprs.ty_of(value) else {
        return Vec::new();
    };
    let borrowed = typed.borrows_value(value);
    let value_ty = reference_pointee_for_type(value_ty, Some(&typed.type_registry))
        .unwrap_or(value_ty.clone());
    let value_ty = typed.type_registry.resolved_definition_for_type(&value_ty);
    let value_targets = typed.exprs.target_group_of(value).cloned().flatten();
    let (fields, variant_name, subst) = match &value_ty {
        Ty::Record { fields, .. } => (fields, None, None),
        Ty::Union {
            variants,
            resolved_params,
            ..
        } => {
            let Some(variant) = variants.iter().find(|variant| variant.name == tag_name) else {
                return Vec::new();
            };
            (
                &variant.fields,
                Some(variant.name),
                resolved_params
                    .as_ref()
                    .map(|args| DepSubst::from_ty_args(args)),
            )
        }
        _ => return Vec::new(),
    };
    let projected_base = variant_name
        .zip(value_targets.as_ref())
        .map(|(name, targets)| {
            targets.projected(|target| ReferenceTargetGroup::Variant {
                base: Box::new(target.clone()),
                name,
            })
        });
    let mut places = Vec::new();
    for (slot, (field_name, binding_name)) in field_bindings.iter().enumerate() {
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
        if !borrowed {
            let place = typed.add_place(crate::typed::TypedPlace {
                binder: BinderId::new(
                    typed.file_id.0,
                    BinderOwner::Pattern {
                        expression: value.0,
                        slot: slot as u32,
                    },
                ),
                name: *binding_name,
                parent: None,
                projection: None,
                mutable: true,
                explicit_contract: true,
                ty: field_type.clone(),
            });
            env.places.insert(*binding_name, place);
            let version = typed.add_place_version(
                place,
                crate::typed::PlaceVersionOrigin::Initializer(value),
                field_type.clone(),
                None,
            );
            env.place_versions.insert(*binding_name, version);
            places.push(place);
        }
        env.types.insert(*binding_name, field_type);
        env.locals.insert(*binding_name);
        if borrowed {
            env.constants.insert(*binding_name);
        } else {
            env.constants.remove(binding_name);
        }
        let base_targets = projected_base.as_ref().or(value_targets.as_ref());
        if let Some(base_targets) = base_targets {
            env.set_target_group(
                *binding_name,
                Some(
                    base_targets.projected(|target| ReferenceTargetGroup::Field {
                        base: Box::new(target.clone()),
                        index,
                    }),
                ),
            );
        }
    }
    places
}

fn is_reference_type(typed: &TypedFileAst, ty: &Ty) -> bool {
    typed.type_registry.reference_former_for_type(ty).is_some()
}

fn result_names_for_expr(
    typed: &TypedFileAst,
    scope: &ExprLowerScope<'_>,
    expr: ExprId,
) -> Vec<Option<Intern<String>>> {
    let Some(TypedExprKind::FnCall { target, .. }) = typed.exprs.kind_of(expr) else {
        return Vec::new();
    };
    if let Some(bind) = typed.defs.get(target)
        && !bind.named_results.is_empty()
    {
        return bind
            .named_results
            .iter()
            .map(|result| result.name)
            .collect();
    }
    scope
        .callable_signatures
        .get(target)
        .map(|signature| signature.results.iter().map(|result| result.name).collect())
        .unwrap_or_default()
}

pub(crate) fn lower_expr_kind(
    typed: &mut TypedFileAst,
    expr: &Typed<Expr>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
    reserved_expr_id: Option<ExprId>,
) -> TypedExprKind {
    let normalized;
    let expr = if let Expr::Bind(bind) = &expr.value {
        if bind.params.is_none() {
            if let BindValue::Body { exprs, ret } = &bind.value {
                let mut items = exprs.clone();
                items.push(Typed::infer(Expr::Return(ret.clone()), ret.span_id));
                let mut bind = bind.clone();
                bind.value =
                    BindValue::Expr(Box::new(Typed::infer(Expr::Block(items), expr.span_id)));
                normalized = Typed::infer(Expr::Bind(bind), expr.span_id);
                &normalized
            } else {
                expr
            }
        } else {
            expr
        }
    } else {
        expr
    };
    match &expr.value {
        Expr::Block(items) => {
            let mut block_env = env.clone();
            let mut statements = Vec::new();
            let mut value = None;
            for (index, item) in items.iter().enumerate() {
                let child = scope.statement();
                let item_scope = if scope.value_required && index + 1 == items.len() {
                    scope
                } else {
                    &child
                };
                let lowered = lower_typed_expr(typed, item, item_scope, &mut block_env);
                if scope.value_required && index + 1 == items.len() {
                    value = Some(lowered);
                } else {
                    statements.push(lowered);
                }
            }
            for (name, place) in env.places.clone() {
                if block_env.places.get(&name) == Some(&place) {
                    macro_rules! copy_binding {
                        ($($field:ident),+ $(,)?) => {$(
                            if let Some(value) = block_env.$field.get(&name) {
                                env.$field.insert(name, value.clone());
                            } else { env.$field.remove(&name); }
                        )+};
                    }
                    copy_binding!(
                        types,
                        place_versions,
                        const_values,
                        target_groups,
                        result_evidence,
                        projected_result_evidence,
                        result_names,
                        result_target_groups
                    );
                }
            }
            env.integer_domains = block_env.integer_domains;
            env.union_members = block_env.union_members;
            env.projected_versions = block_env.projected_versions;
            TypedExprKind::Block { statements, value }
        }
        Expr::Return(ret) => {
            let result = match ret.values.as_slice() {
                [value] => value.clone(),
                values => Typed::infer(Expr::ResultValues(values.to_vec()), ret.span_id),
            };
            let value = if ret.values.is_empty() {
                None
            } else {
                Some(lower_typed_expr(
                    typed,
                    &result,
                    &scope.child().with_expected(scope.return_ty),
                    env,
                ))
            };
            TypedExprKind::Return {
                value,
                postcondition: crate::typed::TypedPostcondition::from_return(ret, &env.types)
                    .map(Box::new),
            }
        }
        Expr::Emit { code, arguments } => {
            let specification = lower_typed_expr(typed, code, &scope.child(), env);
            let signature = crate::emission::signature(
                &typed.exprs.ty[specification.as_usize()],
                &typed.type_registry,
            );
            let record = lower_typed_expr(
                typed,
                arguments,
                &scope.with_expected(signature.as_ref().map(|(input, _)| input)),
                env,
            );
            let ty = typed.exprs.ty[record.as_usize()].clone();
            let fields = crate::emission::operand_fields(&ty, &typed.type_registry);
            let operands = fields
                .into_iter()
                .enumerate()
                .map(|(index, (name, ty))| {
                    let id = super::lower_exprs::append_derived_expr(
                        typed,
                        record,
                        TypedExprKind::AggregateGet {
                            base: record,
                            index,
                        },
                        ty,
                    );
                    (name, id)
                })
                .collect();
            TypedExprKind::Emit {
                specification,
                operands,
                record,
                state: crate::assembly::AssemblyState::Pending,
            }
        }
        Expr::Lit(lit) => TypedExprKind::Lit(lit.clone()),

        Expr::Binary(binary) => {
            if let Some(relation) = ast::ProofRelation::for_bin_op(&binary.op) {
                let (lhs, rhs) =
                    lower_numeric_operands(typed, &binary.lhs, &binary.rhs, scope, env);
                if [lhs, rhs].into_iter().any(|operand| {
                    typed
                        .type_registry
                        .integer_validity_for_type(&typed.exprs.ty[operand.index()])
                        .is_none()
                }) {
                    return TypedExprKind::InvalidOperator {
                        op: binary.op.clone(),
                        lhs,
                        rhs,
                        error: crate::operator::OperatorResolutionError::OperandDoesNotProvide,
                    };
                }
                let signed = [lhs, rhs].into_iter().any(|operand| {
                    typed
                        .type_registry
                        .integer_operation_interpretation_for_type(&typed.exprs.ty[operand.index()])
                        == Some(IntegerInterpretation::Signed)
                });
                let predicate = match relation {
                    ast::ProofRelation::Equal => ComparisonPredicate::Equal,
                    ast::ProofRelation::Less => ComparisonPredicate::Less,
                    ast::ProofRelation::LessOrEqual => ComparisonPredicate::LessOrEqual,
                    ast::ProofRelation::Greater => ComparisonPredicate::Greater,
                    ast::ProofRelation::GreaterOrEqual => ComparisonPredicate::GreaterOrEqual,
                    ast::ProofRelation::NotEqual => unreachable!(),
                };
                return TypedExprKind::IntrinsicCall {
                    op: IntrinsicOp::BitsCompare {
                        interpretation: if signed {
                            IntegerComparison::Signed
                        } else {
                            IntegerComparison::Unsigned
                        },
                        predicate,
                    },
                    args: vec![lhs, rhs],
                };
            }
            let lhs = lower_typed_expr(
                typed,
                &binary.lhs,
                &scope.without_literal_materialization(),
                env,
            );
            let rhs = lower_typed_expr(
                typed,
                &binary.rhs,
                &scope.without_literal_materialization(),
                env,
            );
            let lhs = read_anonymous_integer_operand(typed, lhs);
            let rhs = read_anonymous_integer_operand(typed, rhs);
            let mut lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
            let mut rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            if let Some(expected) = scope.expected_ty
                && typed
                    .type_registry
                    .integer_validity_for_type(expected)
                    .is_some()
            {
                if matches!(lhs_ty, Ty::UnresolvedLiteral(_)) {
                    materialize_integer_operand(typed, lhs, Some(expected));
                    lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
                }
                if matches!(rhs_ty, Ty::UnresolvedLiteral(_)) {
                    materialize_integer_operand(typed, rhs, Some(expected));
                    rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
                }
            }
            if matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                && !matches!(rhs_ty, Ty::UnresolvedLiteral(_))
            {
                let expected = (!matches!(rhs_ty, Ty::AnonymousInteger { .. })).then_some(&rhs_ty);
                materialize_integer_operand(typed, lhs, expected);
                lhs_ty = typed.exprs.ty_of(lhs).cloned().unwrap_or(Ty::Unit);
            } else if matches!(rhs_ty, Ty::UnresolvedLiteral(_))
                && !matches!(lhs_ty, Ty::UnresolvedLiteral(_))
            {
                let expected = (!matches!(lhs_ty, Ty::AnonymousInteger { .. })).then_some(&lhs_ty);
                materialize_integer_operand(typed, rhs, expected);
                rhs_ty = typed.exprs.ty_of(rhs).cloned().unwrap_or(Ty::Unit);
            } else if is_numeric_non_comparison_op(&binary.op)
                && matches!(lhs_ty, Ty::UnresolvedLiteral(_))
                && matches!(rhs_ty, Ty::UnresolvedLiteral(_))
            {
                materialize_integer_operand(typed, lhs, Some(&anonymous_integer_type()));
                materialize_integer_operand(typed, rhs, Some(&anonymous_integer_type()));
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
            match scope
                .operator_registry
                .resolve(binary.op.symbol(), &lhs_ty, &rhs_ty)
            {
                Ok((target, signature)) => TypedExprKind::FnCall {
                    target,
                    args: Some(vec![lhs, rhs]),
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
                        materialize_integer_operand(typed, lhs, Some(&anonymous));
                        materialize_integer_operand(typed, rhs, Some(&anonymous));
                    }
                    let anonymous_op = if lhs_is_anonymous_int && rhs_is_anonymous_int {
                        let interpretation = integer_binary_interpretation(
                            typed,
                            role,
                            [&typed.exprs.ty[lhs.index()], &typed.exprs.ty[rhs.index()]],
                        );
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
                    TypedExprKind::InvalidOperator {
                        op: binary.op.clone(),
                        lhs,
                        rhs,
                        error,
                    }
                }
            }
        }

        Expr::FnCall(fn_call) => lower_fn_call(typed, fn_call, scope, env, reserved_expr_id, None),

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
                &typed.type_registry,
            );
            let disc = result_alternative_discriminant(scope.expected_ty, variant_id.name)
                .unwrap_or_else(|| {
                    resolve_discriminant(
                        &variant_id,
                        scope.variant_map,
                        scope.expected_ty,
                        &typed.type_registry,
                    )
                });
            let field_names = extract_tag_call_field_names(&tag_call.args);
            let member = type_sum_member(scope.expected_ty, tag_call.name, &typed.type_registry);
            let expected_fields = member
                .as_ref()
                .or(scope.expected_ty)
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
                    scope.tag_types.get(&variant_id.union.0).and_then(|ty| {
                        match typed.type_registry.resolved_definition_for_type(ty) {
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
                    let argument = lower_call_arg(typed, arg, &scope.with_expected(expected), env);
                    if let Some(expected) = expected.filter(|expected| {
                        typed
                            .type_registry
                            .integer_validity_for_type(expected)
                            .is_some()
                    }) {
                        let mut flaws = Vec::new();
                        super::bounds_check::check_call_argument(
                            typed, argument, expected, &mut flaws,
                        );
                        let span = typed.exprs.span[argument.index()];
                        for diagnostic in flaws {
                            let diagnostic = diagnostic.at_span_id(span, &typed.span_table);
                            typed.exprs.facts.semantic_facts.flaws[argument.index()]
                                .push(diagnostic);
                        }
                    }
                    let argument = expected.map_or(argument, |expected| {
                        typed.own_storage_argument(argument, expected)
                    });
                    expected.map_or(argument, |expected| {
                        typed.borrow_call_argument(argument, expected)
                    })
                })
                .collect();
            TypedExprKind::TagCall {
                variant_id,
                discriminant: disc,
                args: Some(args),
                field_names,
            }
        }

        Expr::DomainValue(domain) => TypedExprKind::Domain(Box::new((**domain).clone())),
        Expr::TypeValue(_) | Expr::DomainOf(_) | Expr::IntegerDomain(_) => TypedExprKind::Invalid,
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
                &typed.type_registry,
            );
            let disc = result_alternative_discriminant(scope.expected_ty, variant_id.name)
                .unwrap_or_else(|| {
                    resolve_discriminant(
                        &variant_id,
                        scope.variant_map,
                        scope.expected_ty,
                        &typed.type_registry,
                    )
                });
            TypedExprKind::TagCall {
                variant_id,
                discriminant: disc,
                args: Some(vec![]),
                field_names: Vec::new(),
            }
        }

        Expr::Bind(bind) => {
            let argument_context =
                super::type_arguments::TypeArgumentContext::new(scope.tag_params, scope.tag_decls);
            for (span, diagnostic) in argument_context.bind_diagnostics(bind) {
                typed
                    .declaration_flaws
                    .push((span, diagnostic.at_span_id(span, &typed.span_table)));
            }
            // Register constant bindings for the `ReassignConstant` check.
            if bind.is_constant() {
                env.constants.insert(bind.name);
            }

            if bind.params.is_some() {
                let bind_id = reserved_expr_id.expect("local callables reserve an expression ID");
                let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(bind_id.0));
                let expected = TypeEnv::new(scope.tag_types)
                    .with_registry(&typed.type_registry)
                    .with_tag_params(scope.tag_params)
                    .with_tag_declaration_lookup(scope.tag_decls)
                    .resolve_result_alternatives(&bind.anonymous_result_alternatives)
                    .or_else(|| local_callable_result_type(bind, binder, scope.tag_decls))
                    .or_else(|| {
                        bind_explicit_ty_in_scope(
                            bind,
                            &typed.type_registry,
                            scope.tag_types,
                            scope.tag_params,
                            scope.tag_decls,
                        )
                    });
                let has_explicit_contract = expected.is_some();
                let parameters = super::declare::resolve_param_types(
                    bind,
                    &typed.type_registry,
                    scope.tag_types,
                    scope.tag_params,
                    scope.tag_decls,
                    binder,
                    &std::collections::HashMap::new(),
                );
                let defaults: Vec<_> = bind
                    .params
                    .iter()
                    .flat_map(|params| params.values())
                    .map(|parameter| match &parameter.kind {
                        ParameterKind::Default(value) => Some((**value).clone()),
                        _ => None,
                    })
                    .collect();
                if let Some(expected) = &expected {
                    env.local_callables.insert(
                        bind.name,
                        LocalCallable {
                            definition: bind_id,
                            return_type: expected.clone(),
                            parameters: parameters.clone(),
                            defaults: defaults.clone(),
                        },
                    );
                }
                let mut callable_env = env.clone();
                for (slot, (name, ty)) in parameters.iter().enumerate() {
                    callable_env.remove_binding(*name);
                    callable_env.types.insert(*name, ty.clone());
                    callable_env.locals.insert(*name);
                    let place = typed.add_place(crate::typed::TypedPlace {
                        binder: BinderId::new(
                            typed.file_id.0,
                            BinderOwner::LocalParameter {
                                expression: bind_id.0,
                                slot: u32::try_from(slot).expect("local parameter slot fits u32"),
                            },
                        ),
                        name: *name,
                        parent: None,
                        projection: None,
                        mutable: matches!(
                            bind.param_conventions.get(name),
                            Some(ParamConvention::Own | ParamConvention::Mutate)
                        ),
                        explicit_contract: true,
                        ty: ty.clone(),
                    });
                    let version = typed.add_place_version(
                        place,
                        crate::typed::PlaceVersionOrigin::Parameter,
                        ty.clone(),
                        None,
                    );
                    callable_env.places.insert(*name, place);
                    callable_env.place_versions.insert(*name, version);
                }
                typed.local_callable_parameters.insert(
                    bind_id,
                    parameters
                        .iter()
                        .map(|(name, ty)| crate::typed::LocalCallableParameter {
                            name: *name,
                            ty: ty.clone(),
                            convention: bind
                                .param_conventions
                                .get(name)
                                .copied()
                                .unwrap_or_default(),
                        })
                        .collect(),
                );
                let mut callable_scope = scope.with_expected(expected.as_ref());
                callable_scope.return_ty = expected.as_ref();
                callable_scope.value_required = true;
                let is_block_body = matches!(
                    &bind.value,
                    BindValue::Expr(expr) if matches!(expr.value, Expr::Block(_))
                );
                let allows_fallthrough = is_block_body
                    && expected
                        .as_ref()
                        .is_none_or(super::return_types::has_no_results);
                if allows_fallthrough {
                    callable_scope = callable_scope.statement();
                }
                for (name, ty) in &parameters {
                    if let Some(evidence) = super::lower_when::parameter_proposition_evidence(
                        typed,
                        ty,
                        bind.name_span,
                        &callable_scope,
                        &mut callable_env,
                    ) {
                        callable_env.result_evidence.insert(*name, evidence);
                    }
                }
                let (mut stmts, mut body, has_value) =
                    lower_local_bind_body(typed, &bind.value, &callable_scope, &mut callable_env);
                if is_block_body
                    && !crate::analysis::expression_returns(typed.resolved_program(), body)
                {
                    if allows_fallthrough {
                        super::return_types::validate_unit_fallthrough(typed, bind, body);
                        stmts.push(body);
                        body = lower_typed_expr(
                            typed,
                            &Typed::infer(Expr::Block(Vec::new()), bind.name_span),
                            &callable_scope,
                            &mut callable_env,
                        );
                    } else {
                        typed.exprs.facts.semantic_facts.flaws[bind_id.index()].push(
                            // type-missing-return
                            Diagnostic::new(
                                ResultCode::ReturnMissing,
                                "block-bodied functions with results require an explicit return on every path",
                            )
                            .at_span_id(bind.name_span, &typed.span_table),
                        );
                    }
                }
                super::return_types::validate_proposition_returns(
                    typed,
                    bind,
                    &BindBody::Body {
                        exprs: stmts.clone(),
                        ret: has_value.then_some(body),
                    },
                );
                let inferred_integer_return_type =
                    if expected.is_none() && super::return_types::infers_return_type(bind) {
                        super::return_types::infer_integer_return_type(
                            typed,
                            bind,
                            &BindBody::Body {
                                exprs: stmts.clone(),
                                ret: has_value.then_some(body),
                            },
                        )
                    } else {
                        None
                    };
                if let Some(return_type) = &inferred_integer_return_type {
                    super::return_types::validate_integer_return_representation(
                        typed,
                        bind,
                        return_type,
                    );
                }
                let return_type = expected
                    .or(inferred_integer_return_type)
                    .or_else(|| typed.exprs.ty_of(body).cloned())
                    .unwrap_or(Ty::Unit);
                env.local_callables.insert(
                    bind.name,
                    LocalCallable {
                        definition: bind_id,
                        return_type: return_type.clone(),
                        parameters,
                        defaults,
                    },
                );
                env.types.insert(bind.name, return_type.clone());
                let place = typed.add_place(crate::typed::TypedPlace {
                    binder,
                    name: bind.name,
                    parent: None,
                    projection: None,
                    mutable: !bind.is_constant(),
                    explicit_contract: has_explicit_contract,
                    ty: return_type,
                });
                env.places.insert(bind.name, place);
                let version = typed.add_place_version(
                    place,
                    crate::typed::PlaceVersionOrigin::Initializer(body),
                    env.types[&bind.name].clone(),
                    typed.exprs.integer_knowledge_of(body).cloned().flatten(),
                );
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
                        let expected = env.places.get(&bind.name).and_then(|place| {
                            let info = &typed.places[place.0 as usize];
                            info.explicit_contract.then(|| {
                                typed
                                    .borrowed_parameter_referent(*place)
                                    .unwrap_or(&info.ty)
                                    .clone()
                            })
                        });
                        if let ast::BindOperator::Compound(operator) = &bind.operator {
                            let lhs = Typed::infer(
                                Expr::FnCall(ast::expr::FnCall {
                                    syntax: ast::CallSyntax::Path,
                                    definition: None,
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
                                &scope.child().with_expected(expected.as_ref()),
                                env,
                            )
                        } else {
                            lower_typed_expr(
                                typed,
                                inner,
                                &scope.child().with_expected(expected.as_ref()),
                                env,
                            )
                        }
                    }
                    BindValue::Body { exprs, ret } => {
                        let mut lowered: Vec<ExprId> = exprs
                            .iter()
                            .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                            .collect();
                        if !ret.values.is_empty() {
                            let row =
                                Typed::infer(Expr::ResultValues(ret.values.clone()), ret.span_id);
                            lower_typed_expr(typed, &row, scope, env)
                        } else {
                            ret.values
                                .first()
                                .map(|e| lower_typed_expr(typed, e, &scope.child(), env))
                                .or_else(|| lowered.pop())
                                .unwrap_or(ExprId(0))
                        }
                    }
                    _ => ExprId(0),
                };
                if let Some(evidence) = typed.exprs.value_evidence_of(value).cloned() {
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
                let names = result_names_for_expr(typed, scope, value);
                if names.is_empty() {
                    env.result_names.remove(&bind.name);
                } else {
                    env.result_names.insert(bind.name, names);
                }
                let result_targets = typed
                    .exprs
                    .result_target_groups_of(value)
                    .cloned()
                    .unwrap_or_default();
                env.set_result_target_groups(bind.name, Some(result_targets));
                if let Some(place) = env.places.get(&bind.name).copied()
                    && typed.borrowed_parameter_referent(place).is_none()
                {
                    let predecessor = env.place_versions.get(&bind.name).copied();
                    let version = typed.add_place_version(
                        place,
                        crate::typed::PlaceVersionOrigin::Rebind { value, predecessor },
                        typed.places[place.0 as usize].ty.clone(),
                        typed.exprs.integer_knowledge_of(value).cloned().flatten(),
                    );
                    env.set_place_version(bind.name, version);
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
                let expected = bind_explicit_ty_in_scope(
                    bind,
                    &typed.type_registry,
                    scope.tag_types,
                    scope.tag_params,
                    scope.tag_decls,
                );
                let (stmts, mut body, has_value) = lower_local_bind_body(
                    typed,
                    &bind.value,
                    &scope.child().with_expected(expected.as_ref()),
                    env,
                );

                let bind_id = reserved_expr_id.expect("local binds reserve an expression ID");
                let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(bind_id.0));
                let initializer_ty = has_value
                    .then(|| typed.exprs.ty_of(body).cloned())
                    .flatten();
                let declared_ty = super::lower_ty::bind_local_explicit_ty(
                    bind,
                    &typed.type_registry,
                    scope.tag_types,
                    scope.tag_params,
                    scope.tag_decls,
                    binder,
                    initializer_ty.as_ref(),
                )
                .unwrap_or_else(|diagnostic| {
                    typed.declaration_flaws.push((
                        bind.name_span,
                        diagnostic.at_span_id(bind.name_span, &typed.span_table),
                    ));
                    expected.clone()
                });
                if let Some(Ty::AnonymousInteger { validity }) = &declared_ty
                    && !typed.type_registry.is_target_dependent_validity(validity)
                    && let Err(error) = validity.resolve_runtime_representation(
                        &ast::integer::IntegerRepresentationRule::InferFromValidity,
                    )
                {
                    typed.declaration_flaws.push((
                        bind.name_span,
                        Diagnostic::new(
                            error.diagnostic_code(),
                            "integer binding requires finite bounds and a supported runtime width",
                        )
                        .at_span_id(bind.name_span, &typed.span_table),
                    ));
                }
                if let Some(surface) = bind.return_tag.as_deref()
                    && let Some(diagnostic) = TypeEnv::new(scope.tag_types)
                        .with_registry(&typed.type_registry)
                        .with_tag_params(scope.tag_params)
                        .with_tag_declaration_lookup(scope.tag_decls)
                        .integer_domain_diagnostic(surface, &typed.span_table)
                {
                    typed.declaration_flaws.push((surface.span_id, diagnostic));
                }
                if has_value && let Some(expected) = &declared_ty {
                    if matches!(expected, Ty::AnonymousInteger { .. }) {
                        body = read_integer_operand(typed, body);
                    }
                    super::lower_ty::validate_domain_annotation(
                        typed,
                        bind,
                        body,
                        expected,
                        Some(bind_id),
                    );
                }
                let has_explicit_contract = declared_ty.is_some();
                if has_value
                    && let Some(explicit) = &declared_ty
                    && body.as_usize() < typed.exprs.ty.len()
                {
                    if !matches!(explicit, Ty::AnonymousInteger { .. }) {
                        typed.exprs.ty[body.as_usize()] = explicit.clone();
                    }
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
                        typed.exprs.facts.const_facts.const_value[body.as_usize()] = None;
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
                let place = typed.add_place(crate::typed::TypedPlace {
                    binder,
                    name: bind.name,
                    parent: None,
                    projection: None,
                    mutable: !bind.is_constant(),
                    explicit_contract: has_explicit_contract,
                    ty: local_ty.clone(),
                });
                env.places.insert(bind.name, place);
                let version = typed.add_place_version(
                    place,
                    if has_value {
                        crate::typed::PlaceVersionOrigin::Initializer(body)
                    } else {
                        crate::typed::PlaceVersionOrigin::Declared
                    },
                    local_ty.clone(),
                    has_value
                        .then(|| typed.exprs.integer_knowledge_of(body).cloned().flatten())
                        .flatten(),
                );
                env.place_versions.insert(bind.name, version);
                let target_group = if is_declared_ref || is_reference_type(typed, &local_ty) {
                    typed.exprs.target_group_of(body).cloned().flatten()
                } else {
                    Some(ReferenceTargetSet::singleton(ReferenceTargetGroup::Local(
                        place,
                    )))
                };
                if let Some(target_group) = target_group {
                    env.set_target_group(bind.name, Some(target_group));
                } else {
                    env.set_target_group(bind.name, None);
                }
                if has_value
                    && let Some(cv) = typed.exprs.const_value_of(body).and_then(Option::as_ref)
                {
                    env.const_values.insert(bind.name, cv.clone());
                }
                if has_value && let Some(evidence) = typed.exprs.value_evidence_of(body).cloned() {
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
                let names = if has_value {
                    result_names_for_expr(typed, scope, body)
                } else {
                    Default::default()
                };
                if names.is_empty() {
                    env.result_names.remove(&bind.name);
                } else {
                    env.result_names.insert(bind.name, names);
                }
                let result_targets = if has_value {
                    {
                        typed
                            .exprs
                            .result_target_groups_of(body)
                            .cloned()
                            .unwrap_or_default()
                    }
                } else {
                    Default::default()
                };
                env.set_result_target_groups(bind.name, Some(result_targets));

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
            if let Some(subject) = subject_id {
                let _ = super::lower_exprs::ensure_temporary_owner(typed, env, subject);
            }

            let subject_ty = subject_id.and_then(|id| typed.exprs.ty_of(id).cloned());
            let subject_targets =
                subject_id.and_then(|id| typed.exprs.target_group_of(id).cloned().flatten());

            let first_join = typed.place_versions.len();
            let mut arm_scope = scope.with_expected(scope.expected_ty);
            if let (Some(subject), Some(source)) = (subject_id, &when_expr.subject) {
                arm_scope.when_subject = Some((source.span_id, subject));
            }
            let mut arms = lower_all_when_arms(
                typed,
                &when_expr.arms,
                &arm_scope,
                env,
                subject_ty.as_ref(),
                subject_targets.as_ref(),
                subject_id,
            );
            if scope.value_required && scope.expected_ty.is_none() {
                super::when_values::materialize_inferred_union(typed, &mut arms);
            }
            let mut place_joins = root_place_versions_since(typed, first_join, |origin| {
                matches!(origin, crate::typed::PlaceVersionOrigin::Join(_))
            });
            place_joins.retain(|version| {
                let place = typed.place_versions[version.0 as usize].place;
                let name = typed.places[place.0 as usize].name;
                env.places.get(&name) == Some(&place)
                    && env.place_versions.get(&name) == Some(version)
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
            let (mut branch_env, owned_bindings) =
                super::lower_when::with_condition_locals(typed, &incoming, &condition, scope);
            super::lower_when::refine_integer_locals_from_condition(
                typed,
                &condition,
                true,
                &mut branch_env,
            );
            let branch_scope = scope.child();
            let exit_scope = scope.with_expected(scope.expected_ty);
            let stmts: Vec<ExprId> = if_expr
                .body
                .iter()
                .map(|e| {
                    let expr_scope = if matches!(e.value, Expr::If(_) | Expr::Loop(_)) {
                        &exit_scope
                    } else {
                        &branch_scope
                    };
                    lower_typed_expr(typed, e, expr_scope, &mut branch_env)
                })
                .collect();
            let ret = match if_expr.ret.values.as_slice() {
                [] => None,
                [value] => Some(lower_typed_expr(typed, value, &exit_scope, &mut branch_env)),
                values => {
                    let value =
                        Typed::infer(Expr::ResultValues(values.to_vec()), if_expr.ret.span_id);
                    Some(lower_typed_expr(
                        typed,
                        &value,
                        &exit_scope,
                        &mut branch_env,
                    ))
                }
            };
            if ret.is_some() {
                super::lower_when::refine_integer_locals_from_condition(
                    typed, &condition, false, env,
                );
            }
            TypedExprKind::If(Box::new(TypedIfExpr {
                condition,
                stmts,
                ret,
                postcondition: crate::typed::TypedPostcondition::from_return(
                    &if_expr.ret,
                    &branch_env.types,
                ),
                owned_bindings,
                place_joins: Vec::new(),
                body_span: SubSpan::new(SpanId::INVALID),
            }))
        }

        Expr::Loop(loop_enum) => match loop_enum {
            ast::Loop::While(while_loop) => {
                let incoming = env.clone();
                super::lower_exprs::prepare_loop_place_versions(typed, env);
                let condition =
                    super::lower_when::lower_condition(typed, &while_loop.condition, scope, env);
                let (mut body_env, owned_bindings) =
                    super::lower_when::with_condition_locals(typed, env, &condition, scope);
                super::lower_when::refine_integer_locals_from_condition(
                    typed,
                    &condition,
                    true,
                    &mut body_env,
                );
                let stmts: Vec<ExprId> = while_loop
                    .exprs
                    .iter()
                    .map(|e| {
                        let body_scope = scope.statement();
                        lower_typed_expr(typed, e, &body_scope, &mut body_env)
                    })
                    .collect();
                let first_phi = typed.place_versions.len();
                join_loop_place_versions(typed, env, &incoming, &body_env);
                let place_phis = root_place_versions_since(typed, first_phi, |origin| {
                    matches!(origin, crate::typed::PlaceVersionOrigin::LoopPhi { .. })
                });
                TypedExprKind::Loop(TypedLoop {
                    kind: TypedLoopKind::While {
                        condition,
                        stmts,
                        owned_bindings,
                    },
                    place_phis,
                    keyword_span: SubSpan::new(SpanId::INVALID),
                })
            }
            ast::Loop::ForIn(for_in) => super::sequence::lower_for(typed, for_in, scope, env),
        },

        Expr::SelfRef => TypedExprKind::SelfRef {
            target: scope
                .current_def_id
                .unwrap_or(DefId::named(Intern::new("self".to_string()))),
        },

        Expr::FormatString(_) => TypedExprKind::Invalid,

        Expr::Range(range) => {
            let start = lower_typed_expr(typed, &range.start, &scope.child(), env);
            let end = lower_typed_expr(typed, &range.end, &scope.child(), env);
            TypedExprKind::Range { start, end }
        }

        Expr::FixedArrayAlloc { init, size } => {
            let init_expected = (|| {
                if let Some(expected) = scope.expected_ty.map(Ty::without_reference_wrappers)
                    && let Some(value) = match expected {
                        Ty::Named { instance, .. } => {
                            let declaration =
                                typed.type_registry.declaration_for_instance(instance)?;
                            let former = declaration.sequence_literal.as_ref()?;
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

                None
            })();
            let inferred_literal_ty = Ty::i64();
            let init_scope = init_expected
                .as_ref()
                .map(|ty| scope.with_expected(Some(ty)))
                .or_else(|| {
                    matches!(init.value, Expr::Lit(Literal::Int(_)))
                        .then(|| scope.with_expected(Some(&inferred_literal_ty)))
                })
                .unwrap_or_else(|| scope.without_literal_materialization());
            let init_id = lower_typed_expr(typed, init, &init_scope, env);
            let size = lower_array_length(typed, size, scope, env);
            TypedExprKind::FixedArrayAlloc {
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
            let places = add_destructure_bindings(typed, value_id, *tag_name, field_bindings, env);
            TypedExprKind::Destructure {
                tag_name: *tag_name,
                value: value_id,
                field_bindings: field_bindings.clone(),
                places,
            }
        }

        Expr::RecordSet {
            base,
            field,
            value,
            operator,
        } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let base_ty = typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit);
            let base_ty =
                reference_pointee_for_type(&base_ty, Some(&typed.type_registry)).unwrap_or(base_ty);
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let sequence_field = if matches!(base_definition, Ty::Sequence(_)) {
                base_definition.named_field(*field)
            } else {
                None
            };
            let expected = match &base_definition {
                Ty::Record { fields, .. } => fields
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, ty)| ty.as_ref().clone()),
                _ => sequence_field.as_ref().map(|(_, ty)| ty.clone()),
            };
            let value_id =
                lower_typed_expr(typed, value, &scope.with_expected(expected.as_ref()), env);
            if let Some((index, _)) = sequence_field {
                return TypedExprKind::AggregateSet {
                    base: base_id,
                    index,
                    value: value_id,
                    operator: operator.as_ref().map(|operator| {
                        resolve_compound_operator(
                            typed,
                            scope,
                            operator,
                            expected.as_ref().unwrap(),
                            value_id,
                        )
                    }),
                };
            }
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
            if matches!(base.value, Expr::Target) {
                return TypedExprKind::TargetField(*field);
            }
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            // Resolve the field index from the base expression's record type.
            let base_ty = typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit);
            let base_ty = reference_pointee_for_type(&base_ty, Some(&typed.type_registry))
                .unwrap_or(base_ty.clone());
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let field_idx = match base_definition.named_field(*field) {
                Some((index, _)) => Some(index),
                None => {
                    let base_kind = typed.exprs.kind_of(base_id);
                    let result_key = match base_kind {
                        Some(TypedExprKind::Bind { name, .. }) => Some(name),
                        Some(TypedExprKind::SelfRef { target }) => Some(&target.0),
                        Some(TypedExprKind::FnCall { target, .. }) => Some(&target.0),
                        _ => None,
                    };
                    if let Some(index) = result_key.and_then(|key| {
                        env.result_names
                            .get(key)
                            .and_then(|names| names.iter().position(|name| *name == Some(*field)))
                    }) {
                        Some(index)
                    } else {
                        let target = match base_kind {
                            Some(TypedExprKind::FnCall { target, .. }) => Some(*target),
                            _ => None,
                        };
                        target.and_then(|target| {
                            let bind = typed.defs.get(&target)?;
                            if let Some(index) = bind
                                .named_results
                                .iter()
                                .position(|result| result.name == Some(*field))
                            {
                                return Some(index);
                            }
                            if let Some(index) =
                                scope
                                    .callable_signatures
                                    .get(&target)
                                    .and_then(|signature| {
                                        signature
                                            .results
                                            .iter()
                                            .position(|result| result.name == Some(*field))
                                    })
                            {
                                return Some(index);
                            }
                            let body = match &bind.body {
                                BindBody::Expr(expr) => Some(*expr),
                                BindBody::Body { exprs, ret } => {
                                    (*ret).or_else(|| exprs.last().copied())
                                }
                                BindBody::Unlowered => None,
                            }?;
                            let tag_id = match typed.exprs.kind_of(body) {
                                Some(TypedExprKind::TagCall { variant_id, .. }) => {
                                    Some(variant_id.union)
                                }
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
                    }
                }
            };
            if field_idx.is_none()
                && let Some(methods) =
                    super::overloads::Overloads::for_member(*field, &base_ty, typed, scope)
            {
                return match methods.member_call(base, expr.span_id, &[]) {
                    Ok(call) => {
                        lower_fn_call(typed, &call, scope, env, reserved_expr_id, Some(base_id))
                    }
                    Err(diagnostic) => {
                        typed.exprs.facts.semantic_facts.flaws
                            [reserved_expr_id.unwrap().as_usize()]
                        .push(diagnostic.at_span_id(expr.span_id, &typed.span_table));
                        TypedExprKind::Invalid
                    }
                };
            }
            if let Some(idx) = field_idx {
                TypedExprKind::AggregateGet {
                    base: base_id,
                    index: idx,
                }
            } else {
                if let Some(TypedExprKind::FnCall { target, .. }) = typed.exprs.kind_of(base_id)
                    && let Some(bind) = typed.defs.get_mut(target)
                {
                    bind.push_flaw(
                        // type-unknown-result-projection
                        Diagnostic::new(
                            ResultCode::UnknownProjection,
                            format!("unknown result projection `{}`", field.as_str()),
                        )
                        .with_arg("name", field.as_str().to_string())
                        .at_span_id(base.span_id, &typed.span_table),
                    );
                }
                if matches!(base_definition, Ty::Record { .. } | Ty::Sequence(_)) {
                    let expr_id = reserved_expr_id.unwrap_or(base_id);
                    typed.exprs.facts.semantic_facts.flaws[expr_id.as_usize()].push(
                        // type-unknown-field
                        Diagnostic::new(
                            MemberCode::Unknown,
                            format!("record has no field `{}`", field.as_str()),
                        )
                        .with_arg("name", field.as_str().to_string())
                        .at_span_id(base.span_id, &typed.span_table),
                    );
                } else if !matches!(base_definition, Ty::Sequence(..)) {
                    let expr_id = reserved_expr_id.unwrap_or(base_id);
                    typed.exprs.facts.semantic_facts.flaws[expr_id.as_usize()].push(
                        // type-projection-requires-aggregate
                        Diagnostic::new(
                            MemberCode::ProjectionRequiresAggregate,
                            format!(
                                "cannot project named result `{}` from `{}`",
                                field.as_str(),
                                base_ty.format_for_hover()
                            ),
                        )
                        .at_span_id(base.span_id, &typed.span_table),
                    );
                }
                TypedExprKind::Lit(Literal::Int(0.into()))
            }
        }

        Expr::AggregateGet { base, index } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            TypedExprKind::AggregateGet {
                base: base_id,
                index: *index,
            }
        }

        Expr::AggregateSet {
            base,
            index,
            value,
            operator,
        } => {
            let base_id = lower_typed_expr(typed, base, &scope.child(), env);
            let base_ty = typed.exprs.ty_of(base_id).cloned().unwrap_or(Ty::Unit);
            let base_ty =
                reference_pointee_for_type(&base_ty, Some(&typed.type_registry)).unwrap_or(base_ty);
            let base_definition = typed.type_registry.resolved_definition_for_type(&base_ty);
            let expected = match base_definition {
                Ty::Record { fields, .. } => fields.get(*index).map(|(_, field)| (**field).clone()),
                Ty::Sequence(fields) => fields.get(*index).map(|field| (*field.ty).clone()),
                Ty::Array { elem, .. } => Some(*elem),
                _ => None,
            };
            let value_id =
                lower_typed_expr(typed, value, &scope.with_expected(expected.as_ref()), env);
            TypedExprKind::AggregateSet {
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

        Expr::SliceGet { buf, start, length } => {
            let buf = lower_typed_expr(typed, buf, &scope.child(), env);
            let start = lower_typed_expr(typed, start, &scope.child(), env);
            let length = lower_typed_expr(typed, length, &scope.child(), env);
            TypedExprKind::SliceGet { buf, start, length }
        }
        Expr::BufGet { buf, index } => {
            let base_id = lower_typed_expr(typed, buf, &scope.child(), env);
            let index_id = lower_typed_expr(typed, index, &scope.child(), env);
            if matches!(typed.exprs.ty_of(base_id), Some(Ty::Sequence(_)))
                && let Some(ConstValue::Int(value)) =
                    &typed.exprs.facts.const_facts.const_value[index_id.as_usize()]
                && let Some(index) = ast::integer::to_usize(*value)
            {
                return TypedExprKind::AggregateGet {
                    base: base_id,
                    index,
                };
            }
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
            let expected = typed
                .exprs
                .ty_of(base_id)
                .map(|ty| super::lower_ty::extract_element_type(ty, Some(&typed.type_registry)));
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
            let resolved_ty = resolve_type_reference(
                ty,
                scope.tag_types,
                &env.types,
                Some(scope.tag_params),
                &typed.type_registry,
            );
            let address_type = if let Expr::TakePtr(value) = &expr.value
                && typed
                    .type_registry
                    .resolved_definition_for_type(&resolved_ty)
                    .is_ptr()
            {
                Some(Ty::Ptr {
                    inner: Box::new(resolve_expr_type(
                        value,
                        &ExprTypeResolverCtx {
                            tag_types: scope.tag_types,
                            variant_map: scope.variant_map,
                            receiver_type: scope.receiver_type,
                            local_var_types: &env.types,
                            fn_return_types: &typed.fn_return_types,
                            type_registry: &typed.type_registry,
                        },
                        None,
                    )),
                })
            } else {
                None
            };
            let inner_scope = address_type
                .as_ref()
                .map(|ty| scope.with_expected(Some(ty)))
                .unwrap_or_else(|| scope.with_anonymous_literal_materialization());
            let inner_id = lower_typed_expr(typed, expr, &inner_scope, env);
            TypedExprKind::Cast {
                expr: inner_id,
                ty: resolved_ty,
            }
        }

        Expr::Target => TypedExprKind::Target,
        Expr::TargetQuery { kind, operand } => TypedExprKind::TargetQuery {
            kind: *kind,
            operand: resolve_type_reference(
                operand,
                scope.tag_types,
                &env.types,
                Some(scope.tag_params),
                &typed.type_registry,
            ),
        },

        Expr::TakePtr(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::TakePtr(inner_id)
        }

        Expr::Ref { inner, mutable, .. } => {
            let inner_id = if let Expr::SliceGet { buf, start, length } = &inner.value {
                let source = lower_typed_expr(typed, buf, &scope.child(), env);
                let reference = Ty::Ref {
                    inner: Box::new(
                        typed.exprs.ty[source.index()]
                            .without_reference_wrappers()
                            .clone(),
                    ),
                    mutable: *mutable,
                };
                let source = typed.borrow_call_argument(source, &reference);
                let start = lower_typed_expr(typed, start, &scope.child(), env);
                let length = lower_typed_expr(typed, length, &scope.child(), env);
                let ty = super::lower_exprs::slice_type(typed, source, length);
                super::lower_exprs::append_derived_expr(
                    typed,
                    source,
                    TypedExprKind::SliceGet {
                        buf: source,
                        start,
                        length,
                    },
                    ty,
                )
            } else {
                let expected = scope.expected_ty.and_then(|ty| match ty {
                    Ty::Ref { inner, .. } => Some(inner.as_ref()),
                    _ => None,
                });
                lower_typed_expr(typed, inner, &scope.with_expected(expected), env)
            };
            TypedExprKind::Ref(inner_id)
        }

        Expr::Deref(inner) => {
            let inner_id = lower_typed_expr(typed, inner, &scope.child(), env);
            TypedExprKind::Deref(inner_id)
        }

        Expr::Negate(inner) => {
            if let Expr::Lit(Literal::Int(value)) = &inner.value {
                TypedExprKind::Lit(Literal::Int(-*value))
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
            TypedExprKind::RecordValue(item_ids)
        }
        Expr::Import(_) => {
            typed.declaration_flaws.push((
                expr.span_id,
                // use-not-resolved
                Diagnostic::new(
                    ImportCode::NotResolved,
                    "local import must be resolved before typechecking",
                )
                .at_span_id(expr.span_id, &typed.span_table),
            ));
            TypedExprKind::Invalid
        }
        Expr::Invalid(_) => TypedExprKind::Invalid,
        Expr::Todo => TypedExprKind::Todo,
        Expr::ResultValues(items) => {
            let item_ids: Vec<ExprId> = items
                .iter()
                .enumerate()
                .map(|(index, e)| {
                    let expected = match scope.expected_ty {
                        Some(Ty::Sequence(fields)) => fields.get(index).map(|field| &*field.ty),
                        _ => None,
                    };
                    lower_typed_expr(typed, e, &scope.with_expected(expected), env)
                })
                .collect();
            if let Some(Ty::Sequence(fields)) = scope.expected_ty {
                for (id, field) in item_ids.iter().zip(fields) {
                    typed.exprs.ty[id.as_usize()] = (*field.ty).clone();
                }
            }
            TypedExprKind::ResultRow(item_ids)
        }

        Expr::ArrayLit(items) => {
            let element_expected = scope
                .expected_ty
                .and_then(|expected| match Ty::without_reference_wrappers(expected) {
                    Ty::Array { elem, .. } => Some(elem.as_ref().clone()),
                    _ => None,
                })
                .or_else(|| {
                    items.first().and_then(|item| {
                        matches!(item.value, Expr::Lit(Literal::Int(_))).then(Ty::i64)
                    })
                });
            let item_ids: Vec<ExprId> = items
                .iter()
                .map(|e| {
                    let item_scope = element_expected
                        .as_ref()
                        .map(|ty| scope.with_expected(Some(ty)))
                        .unwrap_or_else(|| scope.child());
                    lower_typed_expr(typed, e, &item_scope, env)
                })
                .collect();
            TypedExprKind::ArrayLit(item_ids)
        }

        Expr::SequenceLiteral(ast::SequenceLiteral::Elements(items)) => {
            let element_expected = scope
                .expected_ty
                .and_then(|ty| fixed_array_element_type(typed, ty));
            let expected = scope
                .expected_ty
                .filter(|_| element_expected.is_none())
                .map(|ty| typed.type_registry.resolved_definition_for_type(ty));
            let item_ids: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let ty = match &expected {
                        Some(Ty::Sequence(fields)) => {
                            fields.get(index).map(|field| (*field.ty).clone())
                        }
                        Some(Ty::Record { fields, .. }) => {
                            fields.get(index).map(|(_, ty)| (**ty).clone())
                        }
                        _ => element_expected.clone(),
                    }
                    .or_else(|| matches!(item.value, Expr::Lit(Literal::Int(_))).then(Ty::i64));
                    lower_typed_expr(typed, &item.expr, &scope.with_expected(ty.as_ref()), env)
                })
                .collect();
            if matches!(expected, Some(Ty::Record { .. })) {
                return TypedExprKind::RecordValue(item_ids);
            }
            if (items.is_empty() && element_expected.is_none())
                || matches!(expected, Some(Ty::Sequence(_)))
                || items.iter().any(|item| item.name.is_some())
                || item_ids.first().is_some_and(|first| {
                    item_ids
                        .iter()
                        .any(|id| typed.exprs.ty[id.as_usize()] != typed.exprs.ty[first.as_usize()])
                })
            {
                TypedExprKind::ResultRow(item_ids)
            } else {
                TypedExprKind::ArrayLit(item_ids)
            }
        }

        Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => {
            let seed_expected = scope
                .expected_ty
                .and_then(|expected| fixed_array_element_type(typed, expected))
                .or_else(|| matches!(seed.value, Expr::Lit(Literal::Int(_))).then(Ty::i64));
            let seed_scope = seed_expected
                .as_ref()
                .map(|expected| scope.with_expected(Some(expected)))
                .unwrap_or_else(|| scope.child());
            let seed_id = lower_typed_expr(typed, seed, &seed_scope, env);
            let size = lower_array_length(typed, count, scope, env);
            TypedExprKind::FixedArrayAlloc {
                init: seed_id,
                size,
            }
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

fn result_alternative_discriminant(expected: Option<&Ty>, label: Intern<String>) -> Option<usize> {
    match expected? {
        Ty::ResultFamily { alternatives, .. } => alternatives
            .iter()
            .position(|alternative| alternative.label == label),
        Ty::Union { variants, .. } => variants.iter().position(|variant| variant.name == label),
        _ => None,
    }
}

fn local_callable_result_type(
    bind: &Bind,
    binder: BinderId,
    tag_decls: TagDeclarations<'_>,
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
    tag_decls: TagDeclarations<'_>,
    labels: &mut Vec<Intern<String>>,
) {
    match value {
        BindValue::Expr(expr) => collect_result_exit_alternatives(&expr.value, tag_decls, labels),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                if matches!(expr.value, Expr::If(_) | Expr::Loop(_) | Expr::When(_)) {
                    collect_result_exit_alternatives(&expr.value, tag_decls, labels);
                }
            }
            for exit in &ret.values {
                collect_result_exit_alternatives(&exit.value, tag_decls, labels);
            }
        }
        BindValue::Unassigned => {}
    }
}

fn lower_local_bind_body(
    typed: &mut TypedFileAst,
    value: &BindValue,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> (Vec<ExprId>, ExprId, bool) {
    match value {
        BindValue::Expr(expr) => (Vec::new(), lower_typed_expr(typed, expr, scope, env), true),
        BindValue::Body { exprs, ret } => {
            let mut lowered = Vec::new();
            for expr in exprs {
                lowered.push(lower_typed_expr(typed, expr, &scope.statement(), env));
            }
            let body = match ret.values.as_slice() {
                [] => Some(lower_typed_expr(
                    typed,
                    &Typed::infer(Expr::Block(Vec::new()), ret.span_id),
                    scope,
                    env,
                )),
                [exit] => Some(lower_typed_expr(typed, exit, scope, env)),
                values => {
                    let result = Typed::infer(Expr::ResultValues(values.to_vec()), ret.span_id);
                    Some(lower_typed_expr(typed, &result, scope, env))
                }
            };
            match body {
                Some(body) => (lowered, body, true),
                None => (Vec::new(), ExprId(0), false),
            }
        }
        BindValue::Unassigned => (Vec::new(), ExprId(0), false),
    }
}

fn local_argument_order(
    callable: &LocalCallable,
    arguments: &[Typed<Expr>],
) -> Option<Vec<Option<usize>>> {
    assert_eq!(
        callable.parameters.len(),
        callable.defaults.len(),
        "local callable parameters must have aligned defaults"
    );
    let mut order = vec![None; callable.parameters.len()];
    let mut positional = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if let Expr::Bind(bind) = &argument.value
            && bind.params.is_none()
        {
            let slot = callable
                .parameters
                .iter()
                .position(|(name, _)| *name == bind.name)?;
            if order[slot].replace(index).is_some() {
                return None;
            }
        } else {
            positional.push(index);
        }
    }
    let mut positional = positional.into_iter();
    for slot in &mut order {
        if slot.is_none() {
            *slot = positional.next();
        }
    }
    if positional.next().is_some() {
        return None;
    }
    order
        .iter()
        .zip(&callable.defaults)
        .all(|(argument, default)| argument.is_some() || default.is_some())
        .then_some(order)
}

struct LocalCallSite<'a> {
    fn_call: &'a ast::FnCall,
    arguments: &'a [Typed<Expr>],
    target: DefId,
    callable: &'a LocalCallable,
    call_id: ExprId,
}

fn lower_local_call(
    typed: &mut TypedFileAst,
    call: LocalCallSite<'_>,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
) -> TypedExprKind {
    let LocalCallSite {
        fn_call,
        arguments,
        target,
        callable,
        call_id,
    } = call;
    typed
        .local_callable_calls
        .insert(call_id, callable.definition);
    let Some(order) = local_argument_order(callable, arguments) else {
        let required = callable
            .defaults
            .iter()
            .filter(|default| default.is_none())
            .count();
        let code = if arguments.len() < required || arguments.len() > callable.parameters.len() {
            // type-call-argument-count
            DiagnosticCode::Type(TypeCode::Call(CallCode::ArgumentCount))
        } else {
            // type-call-argument-name
            DiagnosticCode::Type(TypeCode::Call(CallCode::ArgumentName))
        };
        typed.exprs.facts.semantic_facts.flaws[call_id.as_usize()].push(
            Diagnostic::new(
                code,
                "arguments do not match the local callable's parameters",
            )
            .at_span_id(fn_call.path.span_id, &typed.span_table),
        );
        let args = arguments
            .iter()
            .map(|argument| lower_call_arg(typed, argument, &scope.child(), env))
            .collect();
        return TypedExprKind::FnCall {
            target,
            args: fn_call.args.is_some().then_some(args),
            substituted_ty: Some(callable.return_type.clone()),
        };
    };
    let supplied: Vec<_> = arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let parameter = order
                .iter()
                .position(|slot| *slot == Some(index))
                .expect("every supplied argument has a parameter slot");
            lower_call_arg(
                typed,
                argument,
                &scope.with_expected(Some(&callable.parameters[parameter].1)),
                env,
            )
        })
        .collect();
    let mut args = Vec::with_capacity(order.len());
    for (parameter, argument) in order.into_iter().enumerate() {
        let expected = &callable.parameters[parameter].1;
        let value = match argument {
            Some(index) => supplied[index],
            None => lower_call_arg(
                typed,
                callable.defaults[parameter]
                    .as_ref()
                    .expect("an omitted argument has a default"),
                &scope.with_expected(Some(expected)),
                env,
            ),
        };
        let value = typed.borrow_call_argument(value, expected);
        args.push(value);
    }
    let mut substitution = crate::subst::DepSubst::new();
    for ((name, _), argument) in callable.parameters.iter().zip(&args) {
        if let Some(value) =
            super::symbolic_values::normal_expr(typed, None, &Default::default(), *argument)
        {
            substitution.consts.insert(*name, value);
        }
    }
    for ((_, expected), value) in callable.parameters.iter().zip(&args) {
        let expected = if matches!(expected.without_reference_wrappers(), Ty::Proposition(_)) {
            substitution.apply_to_ty(expected)
        } else {
            expected.clone()
        };
        let mut flaws = Vec::new();
        super::bounds_check::check_call_argument(typed, *value, &expected, &mut flaws);
        typed.exprs.facts.semantic_facts.flaws[value.as_usize()].extend(flaws);
    }
    TypedExprKind::FnCall {
        target,
        args: Some(args),
        substituted_ty: Some(if matches!(callable.return_type, Ty::Proposition(_)) {
            substitution.apply_to_ty(&callable.return_type)
        } else {
            callable.return_type.clone()
        }),
    }
}

fn lower_fn_call(
    typed: &mut TypedFileAst,
    fn_call: &ast::FnCall,
    scope: &ExprLowerScope<'_>,
    env: &mut FlowState,
    reserved_expr_id: Option<ExprId>,
    mut lowered_receiver: Option<ExprId>,
) -> TypedExprKind {
    let member = if fn_call.syntax == ast::CallSyntax::Receiver {
        Some(fn_call.clone())
    } else if fn_call.definition.is_none()
        && (env.types.contains_key(&fn_call.path.root)
            || env.places.contains_key(&fn_call.path.root)
            || typed.defs.contains_key(&DefId::named(fn_call.path.root))
            || scope
                .callable_signatures
                .contains_key(&DefId::named(fn_call.path.root)))
    {
        fn_call.with_path_receiver()
    } else {
        None
    };
    if let Some(member) = member
        && let Some((source, args)) = member.args.as_deref().and_then(|args| args.split_first())
    {
        let receiver = lower_call_arg(typed, source, &scope.child(), env);
        let ty = typed
            .exprs
            .ty_of(receiver)
            .cloned()
            .unwrap_or_else(|| Ty::Opaque(member.path.root));
        if let Some(methods) =
            super::overloads::Overloads::for_member(member.path.root, &ty, typed, scope)
        {
            return match methods.member_call(source, fn_call.path.span_id, args) {
                Ok(call) => {
                    typed
                        .resolved_member_calls
                        .insert(reserved_expr_id.unwrap(), call.clone());
                    lower_fn_call(typed, &call, scope, env, reserved_expr_id, Some(receiver))
                }
                Err(diagnostic) => {
                    typed.exprs.facts.semantic_facts.flaws[reserved_expr_id.unwrap().as_usize()]
                        .push(diagnostic.at_span_id(fn_call.path.span_id, &typed.span_table));
                    TypedExprKind::Invalid
                }
            };
        }
        lowered_receiver = Some(receiver);
        return lower_fn_call(
            typed,
            &ast::FnCall {
                syntax: ast::CallSyntax::Path,
                ..member
            },
            scope,
            env,
            reserved_expr_id,
            lowered_receiver,
        );
    }
    if fn_call.args.is_none()
        && fn_call.path.segments.is_empty()
        && env.places.contains_key(&fn_call.path.root)
        && !env.local_callables.contains_key(&fn_call.path.root)
    {
        return TypedExprKind::FnCall {
            target: DefId::named(fn_call.path.root),
            args: None,
            substituted_ty: env.types.get(&fn_call.path.root).cloned(),
        };
    }

    let mut target = fn_call
        .definition
        .unwrap_or_else(|| resolve_fn_call_target(&fn_call.path.value, scope.tag_types));
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
        target = DefId::named(Intern::new(parts.join(".")));
    }
    if !typed.defs.contains_key(&target)
        && !scope.callable_signatures.contains_key(&target)
        && !target.0.as_str().contains('.')
        && let Some(receiver) = scope.receiver_type.and_then(Ty::type_name)
    {
        let receiver_target = DefId::named(Intern::new(format!("{receiver}.{}", target.0)));
        if typed.defs.contains_key(&receiver_target)
            || scope.callable_signatures.contains_key(&receiver_target)
        {
            target = receiver_target;
        }
    }
    if !typed.defs.contains_key(&target)
        && !scope.callable_signatures.contains_key(&target)
        && !target.0.as_str().contains('.')
        && !env.local_callables.contains_key(&target.0)
        && let Some(receiver) = call_args.first()
    {
        let receiver = lowered_receiver
            .unwrap_or_else(|| lower_call_arg(typed, receiver, &scope.child(), env));
        lowered_receiver = Some(receiver);
        if let Some(receiver_name) = typed.exprs.ty_of(receiver).and_then(Ty::type_name) {
            let receiver_target =
                DefId::named(Intern::new(format!("{receiver_name}.{}", target.0)));
            if typed.defs.contains_key(&receiver_target)
                || scope.callable_signatures.contains_key(&receiver_target)
            {
                target = receiver_target;
            }
        }
    }
    if fn_call.path.value.segments.is_empty()
        && let Some(callable) = env.local_callables.get(&target.0).cloned()
    {
        return lower_local_call(
            typed,
            LocalCallSite {
                fn_call,
                arguments: call_args,
                target,
                callable: &callable,
                call_id: reserved_expr_id.expect("local calls reserve an expression ID"),
            },
            scope,
            env,
        );
    }
    let overloads = if fn_call.definition.is_some()
        || (fn_call.path.segments.is_empty() && env.types.contains_key(&target.0))
    {
        None
    } else {
        super::overloads::Overloads::find(target, typed, scope)
    };
    let overloads = overloads.and_then(|overloads| {
        if let Some(selected) = overloads.unique_for_arguments(call_args) {
            target = selected;
            None
        } else {
            Some(overloads)
        }
    });
    let selected = if let Some(overloads) = overloads {
        let args: Vec<_> = call_args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                if index == 0
                    && let Some(receiver) = lowered_receiver
                {
                    receiver
                } else {
                    lower_call_arg(typed, arg, &scope.child(), env)
                }
            })
            .collect();
        match overloads.select(call_args, &args, typed) {
            Ok((selected, signature, args)) => {
                target = selected;
                Some((signature, args))
            }
            Err(diagnostic) => {
                let call = reserved_expr_id.expect("function calls reserve an expression ID");
                typed.exprs.facts.semantic_facts.flaws[call.as_usize()]
                    .push(diagnostic.at_span_id(fn_call.path.span_id, &typed.span_table));
                return TypedExprKind::Invalid;
            }
        }
    } else {
        None
    };
    let local_signature = if let Some((signature, _)) = &selected {
        Some(signature.clone())
    } else {
        typed
            .defs
            .get(&target)
            .map(TypedCallableSignature::from)
            .or_else(|| scope.callable_signatures.get(&target).cloned())
    };
    let signature = local_signature.as_ref();
    let args = call_args;
    let mut lowered_args: Vec<ExprId> = if let Some((_, args)) = selected {
        args
    } else {
        let order = signature.and_then(|signature| {
            let names: Vec<_> = args
                .iter()
                .map(|arg| match &arg.value {
                    Expr::Bind(bind) if bind.params.is_none() => Some(bind.name),
                    _ => None,
                })
                .collect();
            crate::staging::argument_order(&names, &signature.params)
        });
        let lowered: Vec<_> = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                let parameter = order
                    .as_ref()
                    .and_then(|order| order.iter().position(|argument| *argument == index))
                    .unwrap_or(index);
                let expected = signature
                    .and_then(|signature| signature.params.get(parameter))
                    .map(|(_, ty)| ty);
                let type_argument = signature
                    .and_then(|signature| signature.param_kinds.get(parameter))
                    .filter(|kind| matches!(kind, ast::ParamKind::Type))
                    .and_then(|_| lower_named_type_argument(typed, arg, scope));
                let argument = if let Some(argument) = type_argument {
                    argument
                } else if index == 0
                    && let Some(receiver) = lowered_receiver
                {
                    receiver
                } else {
                    lower_call_arg(typed, arg, &scope.with_expected(expected), env)
                };
                if signature.is_none() {
                    typed.exprs.facts.semantic_facts.flaws[argument.as_usize()].retain(|flaw| {
                        !matches!(
                            flaw.code,
                            DiagnosticCode::Type(TypeCode::Text(TextCode::RequiresContext))
                                | DiagnosticCode::Type(TypeCode::Sequence(
                                    SequenceCode::LiteralRequiresContext
                                ))
                                | DiagnosticCode::Type(TypeCode::Sequence(
                                    SequenceCode::RepeatRequiresContext
                                ))
                        )
                    });
                }
                argument
            })
            .collect();
        if let Some(order) = order {
            order.into_iter().map(|index| lowered[index]).collect()
        } else {
            lowered
        }
    };
    if let Some(signature) = signature {
        for (argument, (_, expected)) in lowered_args.iter_mut().zip(&signature.params) {
            let value_type = match expected {
                Ty::Ref { inner, .. } => inner.as_ref(),
                ty => ty,
            };
            if !typed.exprs.ty[argument.as_usize()].is_ref() {
                super::lower_exprs::contextualize_literal_value(typed, *argument, value_type);
                contextualize_local_literal(typed, *argument, value_type, env);
            }
            *argument = typed.borrow_call_argument(*argument, expected);
        }
    }
    let has_arg_exprs = fn_call.args.is_some();
    let call_signature = signature.map(|signature| {
        let call_id = reserved_expr_id.expect("function calls reserve an expression ID");
        let binder = BinderId::new(typed.file_id.0, BinderOwner::Expression(call_id.0));
        let mut instantiation = DependentInstantiation::new(signature.dependent_binder, binder);
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
        let by_expected = infer_return_type_from_expected(typed, return_type, scope.expected_ty);
        by_expected
            .or(by_call)
            .or_else(|| Some(return_type.clone()))
    } else {
        None
    };
    TypedExprKind::FnCall {
        target,
        args: lowered,
        substituted_ty,
    }
}

#[cfg(test)]
#[path = "lower_kind_tests.rs"]
mod tests;
