use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::OnceLock;

use ast::{
    BinOp, BinderId, BinderOwner, ConstValue, DependentArgId, Literal,
    NormalExpr as DependentNormalExpr, ParamKind,
};
use internment::Intern;

use crate::ResolvedProgram;
use crate::subst::DepSubst;
use crate::ty::Ty;
use crate::typed::{
    BindBody, CallCapability, DefId, ExprId, TypedExprKind, TypedFileAst, VariantId,
};

const MAX_COMPILE_TIME_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NormalExpr {
    /// A concrete constant value.
    Value(ConstValue),
    /// A named symbol (local or function).
    Var(Intern<String>),
    /// A parameter symbol within a binder.
    Param(DependentArgId),
    /// A unary minus.
    Negate(Box<NormalExpr>),
    /// A binary operator on normalized sub-expressions.
    Binary {
        op: BinOp,
        lhs: Box<NormalExpr>,
        rhs: Box<NormalExpr>,
    },
    /// A supported function call in normalized form.
    FnCall {
        target: DefId,
        args: Vec<NormalExpr>,
    },
    TagCall {
        variant_id: VariantId,
        args: Vec<NormalExpr>,
    },
    /// An aggregate/record field access in residual form.
    FieldGet { base: Box<NormalExpr>, index: usize },
    TargetQuery {
        kind: ast::TargetQueryKind,
        operand: ast::TypeReference,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NormalizeResult {
    /// Fully evaluated compile-time value.
    Value(ConstValue),
    /// Symbolic residual that is still resolved.
    Residual(NormalExpr),
    /// Expression touches runtime dependency (directly or indirectly).
    RuntimeDependency(ExprId),
    /// The expression has no stable symbolic representation.
    Unsupported(ExprId),
}

impl NormalizeResult {
    /// Best-effort conversion into the dependent-type normal form.
    pub fn as_dependent_normal_expr(&self) -> Option<DependentNormalExpr> {
        match self {
            NormalizeResult::Value(value) => Some(DependentNormalExpr::Value(value.clone())),
            NormalizeResult::Residual(form) => form.as_dependent_normal_expr(),
            NormalizeResult::RuntimeDependency(_) | NormalizeResult::Unsupported(_) => None,
        }
    }
}

impl NormalExpr {
    fn as_dependent_normal_expr(&self) -> Option<DependentNormalExpr> {
        match self {
            NormalExpr::Value(value) => Some(DependentNormalExpr::Value(value.clone())),
            NormalExpr::Var(name) => Some(DependentNormalExpr::Var(*name)),
            NormalExpr::Param(param) => Some(DependentNormalExpr::Inferred(*param)),
            NormalExpr::Negate(expr) => expr
                .as_dependent_normal_expr()
                .as_ref()
                .and_then(DependentNormalExpr::as_const_int)
                .map(|value| (-value).into()),
            NormalExpr::Binary { op, lhs, rhs } => {
                let left = lhs.as_dependent_normal_expr()?;
                let right = rhs.as_dependent_normal_expr()?;
                match op {
                    BinOp::Add => Some(DependentNormalExpr::Add(Box::new(left), Box::new(right))),
                    BinOp::Subtract => {
                        Some(DependentNormalExpr::Sub(Box::new(left), Box::new(right)))
                    }
                    BinOp::Multiply => {
                        Some(DependentNormalExpr::Mul(Box::new(left), Box::new(right)))
                    }
                    _ => None,
                }
            }
            NormalExpr::FnCall { target, args } if args.is_empty() => {
                Some(DependentNormalExpr::Var(target.0))
            }
            NormalExpr::TargetQuery { kind, operand } => Some(DependentNormalExpr::TargetQuery {
                kind: *kind,
                operand: operand.clone(),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NormalizeCall {
    target: DefId,
    args: Vec<ConstValue>,
}

pub(super) struct Normalizer<'a> {
    program: ResolvedProgram<'a>,
    owners: OnceLock<HashMap<ExprId, BinderId>>,
}

impl<'a> Normalizer<'a> {
    pub(super) fn new(program: ResolvedProgram<'a>) -> Self {
        Self {
            program,
            owners: OnceLock::new(),
        }
    }

    pub(super) fn normalize(&self, expr_id: ExprId, substitutions: &DepSubst) -> NormalizeResult {
        let mut context = NormalizeContext {
            program: self.program,
            inputs: self,
            depth: 0,
            call_stack: HashSet::new(),
        };
        let substitutions = dependent_substitutions(self.program, substitutions);
        normalize_expr(expr_id, &mut context, &substitutions)
    }
}

struct NormalizeContext<'a, 'n> {
    program: ResolvedProgram<'a>,
    inputs: &'n Normalizer<'a>,
    depth: usize,
    call_stack: HashSet<NormalizeCall>,
}

impl NormalizeContext<'_, '_> {
    fn with_depth<F>(&mut self, expr_id: ExprId, f: F) -> NormalizeResult
    where
        F: FnOnce(&mut Self) -> NormalizeResult,
    {
        self.depth += 1;
        if self.depth > MAX_COMPILE_TIME_DEPTH {
            self.depth -= 1;
            return NormalizeResult::Unsupported(expr_id);
        }
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn owner_for_expr(&self, expr_id: ExprId) -> Option<BinderId> {
        self.inputs
            .owners
            .get_or_init(|| ExpressionOwners::collect(self.program))
            .get(&expr_id)
            .copied()
    }

    fn param_for_name(&self, owner: BinderId, name: Intern<String>) -> Option<DependentArgId> {
        let BinderOwner::Definition(owner_name) = owner.owner else {
            return None;
        };
        let bind = self.program.defs.get(&owner_name)?;
        let slot = bind.params.iter().position(|(param, _)| *param == name)?;
        Some(DependentArgId::new(owner, slot as u32))
    }
}

struct ExpressionOwners<'a> {
    program: ResolvedProgram<'a>,
    values: HashMap<ExprId, BinderId>,
}

impl ExpressionOwners<'_> {
    fn collect(program: ResolvedProgram<'_>) -> HashMap<ExprId, BinderId> {
        let mut owners = ExpressionOwners {
            program,
            values: HashMap::new(),
        };
        owners.build_expr_owner_map();
        owners.values
    }

    fn build_expr_owner_map(&mut self) {
        for bind in self.program.defs.values() {
            let owner = bind.dependent_binder;
            match &bind.body {
                BindBody::Expr(expr) => self.assign_expr_owner(owner, *expr),
                BindBody::Body { exprs, ret, .. } => {
                    for expr in exprs {
                        self.assign_expr_owner(owner, *expr);
                    }
                    if let Some(ret) = ret {
                        self.assign_expr_owner(owner, *ret);
                    }
                }
                BindBody::Unlowered => {}
            }
        }
    }

    fn assign_expr_owner(&mut self, owner: BinderId, expr_id: ExprId) {
        if self.values.contains_key(&expr_id) {
            return;
        }
        self.values.insert(expr_id, owner);

        let Some(kind) = self.program.exprs.kind.get(expr_id.as_usize()) else {
            return;
        };

        if let TypedExprKind::Bind {
            stmts,
            body,
            unassigned,
            ..
        } = kind
        {
            let nested_owner =
                BinderId::new(self.program.file_id.0, BinderOwner::Expression(expr_id.0));
            for stmt in stmts {
                self.assign_expr_owner(nested_owner, *stmt);
            }
            if !unassigned {
                self.assign_expr_owner(nested_owner, *body);
            }
            return;
        }

        if matches!(kind, TypedExprKind::FnCall { args: None, .. }) {
            return;
        }

        let _ = crate::typed::walk_expr_children(kind, &mut |child| {
            self.assign_expr_owner(owner, child);
            ControlFlow::Continue(())
        });
    }
}

/// Normalize a resolved expression into a symbolic normal form.
pub fn normalize(
    typed: &TypedFileAst,
    expr_id: ExprId,
    substitutions: &DepSubst,
) -> NormalizeResult {
    normalize_resolved(typed.resolved_program(), expr_id, substitutions)
}

pub fn normalize_resolved(
    program: ResolvedProgram<'_>,
    expr_id: ExprId,
    substitutions: &DepSubst,
) -> NormalizeResult {
    Normalizer::new(program).normalize(expr_id, substitutions)
}

fn dependent_substitutions(
    program: ResolvedProgram<'_>,
    substitutions: &DepSubst,
) -> HashMap<DependentArgId, NormalizeResult> {
    let mut result = HashMap::new();
    if substitutions.consts.is_empty() {
        return result;
    }
    for bind in program.defs.values() {
        for (slot, (name, _)) in bind.params.iter().enumerate() {
            let Some(value) = substitutions.consts.get(name) else {
                continue;
            };
            result.insert(
                DependentArgId::new(bind.dependent_binder, slot as u32),
                dependent_expr_to_result(value),
            );
        }
    }
    result
}

fn dependent_expr_to_result(expr: &DependentNormalExpr) -> NormalizeResult {
    match expr {
        DependentNormalExpr::Value(value) => NormalizeResult::Value(value.clone()),
        DependentNormalExpr::Var(name) => NormalizeResult::Residual(NormalExpr::Var(*name)),
        DependentNormalExpr::Inferred(param) => {
            NormalizeResult::Residual(NormalExpr::Param(*param))
        }
        DependentNormalExpr::Add(lhs, rhs) => dependent_binary_to_result(BinOp::Add, lhs, rhs),
        DependentNormalExpr::Sub(lhs, rhs) => dependent_binary_to_result(BinOp::Subtract, lhs, rhs),
        DependentNormalExpr::Mul(lhs, rhs) => dependent_binary_to_result(BinOp::Multiply, lhs, rhs),
        DependentNormalExpr::TargetQuery { kind, operand } => {
            NormalizeResult::Residual(NormalExpr::TargetQuery {
                kind: *kind,
                operand: operand.clone(),
            })
        }
    }
}

fn dependent_binary_to_result(
    op: BinOp,
    lhs: &DependentNormalExpr,
    rhs: &DependentNormalExpr,
) -> NormalizeResult {
    let lhs = dependent_expr_to_result(lhs);
    let rhs = dependent_expr_to_result(rhs);
    match (lhs, rhs) {
        (NormalizeResult::Value(lhs), NormalizeResult::Value(rhs)) => {
            if let Some(value) = lhs.eval_binop(&op, &rhs) {
                NormalizeResult::Value(value)
            } else {
                NormalizeResult::Residual(NormalExpr::Binary {
                    op,
                    lhs: Box::new(NormalExpr::Value(lhs)),
                    rhs: Box::new(NormalExpr::Value(rhs)),
                })
            }
        }
        (lhs, rhs) => {
            let lhs = to_normal_expr(lhs);
            let rhs = to_normal_expr(rhs);
            match (lhs, rhs) {
                (Some(lhs), Some(rhs)) => NormalizeResult::Residual(NormalExpr::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }),
                _ => NormalizeResult::Unsupported(ExprId(0)),
            }
        }
    }
}

fn normalize_expr(
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    context.with_depth(expr_id, |context| {
        let Some(expr) = context.program.exprs.kind.get(expr_id.as_usize()) else {
            return NormalizeResult::Unsupported(expr_id);
        };
        let statements = match expr {
            TypedExprKind::Bind { stmts, .. } => stmts.as_slice(),
            TypedExprKind::Block { statements, .. } => statements.as_slice(),
            _ => &[],
        };
        {
            for statement in statements {
                match normalize_expr(*statement, context, substitutions) {
                    NormalizeResult::Value(_) => {}
                    NormalizeResult::Residual(_) => {
                        return NormalizeResult::Unsupported(expr_id);
                    }
                    result @ (NormalizeResult::RuntimeDependency(_)
                    | NormalizeResult::Unsupported(_)) => return result,
                }
            }
        }
        if let Some(value) =
            context.program.exprs.facts.const_facts.const_value[expr_id.as_usize()].clone()
        {
            return NormalizeResult::Value(value);
        }

        match expr {
            TypedExprKind::Block { value, .. } => value.map_or_else(
                || {
                    NormalizeResult::Value(ConstValue::Record {
                        fields: Vec::new().into(),
                    })
                },
                |value| normalize_expr(value, context, substitutions),
            ),
            TypedExprKind::Emit { .. } => NormalizeResult::RuntimeDependency(expr_id),
            TypedExprKind::Lit(Literal::Int(value)) => {
                NormalizeResult::Value(ConstValue::Int(*value))
            }
            TypedExprKind::SelfRef { target } => {
                NormalizeResult::Residual(NormalExpr::Var(target.0))
            }
            TypedExprKind::FnCall {
                target,
                args: Some(args),
                ..
            } => normalize_call(*target, args, expr_id, context, substitutions),
            TypedExprKind::FnCall {
                target, args: None, ..
            } => {
                if let Some(owner) = context.owner_for_expr(expr_id)
                    && let Some(param) = context.param_for_name(owner, target.0)
                {
                    if let Some(value) = substitutions.get(&param) {
                        return value.clone();
                    }
                    return NormalizeResult::Residual(NormalExpr::Param(param));
                }

                NormalizeResult::Residual(NormalExpr::FnCall {
                    target: *target,
                    args: Vec::new(),
                })
            }
            TypedExprKind::Bind {
                body,
                unassigned,
                name,
                ..
            } => {
                if *unassigned {
                    if matches!(context.program.exprs.ty[expr_id.as_usize()], Ty::Unit) {
                        NormalizeResult::Value(ConstValue::Tag {
                            name: *name,
                            qual_path: None,
                            args: Vec::new().into(),
                        })
                    } else {
                        NormalizeResult::Residual(NormalExpr::Var(*name))
                    }
                } else {
                    normalize_expr(*body, context, substitutions)
                }
            }
            TypedExprKind::Binary { op, lhs, rhs } => {
                normalize_binary(*lhs, *rhs, op.clone(), expr_id, context, substitutions)
            }
            TypedExprKind::IntrinsicCall { op, args } => {
                let values: Option<Vec<_>> = args
                    .iter()
                    .map(|arg| match normalize_expr(*arg, context, substitutions) {
                        NormalizeResult::Value(value) => Some(value),
                        _ => None,
                    })
                    .collect();
                let Some(values) = values else {
                    return NormalizeResult::RuntimeDependency(expr_id);
                };
                if let Some(value) = op.fold_comparison(&values) {
                    return NormalizeResult::Value(value);
                }
                if let [left, right] = args.as_slice()
                    && let Some(value) = crate::anonymous_arithmetic::fold(
                        *op,
                        &values,
                        [
                            &context.program.exprs.ty[left.as_usize()],
                            &context.program.exprs.ty[right.as_usize()],
                        ],
                    )
                {
                    return match value {
                        Ok(value) => NormalizeResult::Value(value),
                        Err(_) => NormalizeResult::Unsupported(expr_id),
                    };
                }
                let source_width = args.first().and_then(|arg| {
                    context
                        .program
                        .type_registry
                        .integer_width_for_type(&context.program.exprs.ty[arg.as_usize()])
                });
                let result_width = context
                    .program
                    .type_registry
                    .integer_width_for_type(&context.program.exprs.ty[expr_id.as_usize()])
                    .or_else(|| {
                        matches!(
                            context.program.exprs.ty[expr_id.as_usize()],
                            Ty::Proposition(_)
                        )
                        .then_some(1)
                    });
                match (source_width, result_width) {
                    (Some(source_width), Some(result_width)) => {
                        match op.fold(&values, source_width, result_width) {
                            Ok(Some(value)) => NormalizeResult::Value(value),
                            Ok(None) | Err(_) => NormalizeResult::Unsupported(expr_id),
                        }
                    }
                    _ => NormalizeResult::Unsupported(expr_id),
                }
            }
            TypedExprKind::Negate(inner) => {
                let inner = normalize_expr(*inner, context, substitutions);
                match inner {
                    NormalizeResult::Value(ConstValue::Int(value)) => {
                        NormalizeResult::Value(ConstValue::Int(-value))
                    }
                    NormalizeResult::Value(value) => NormalizeResult::Residual(NormalExpr::Negate(
                        Box::new(NormalExpr::Value(value)),
                    )),
                    NormalizeResult::Residual(expr) => {
                        NormalizeResult::Residual(NormalExpr::Negate(Box::new(expr)))
                    }
                    NormalizeResult::RuntimeDependency(expr) => {
                        NormalizeResult::RuntimeDependency(expr)
                    }
                    NormalizeResult::Unsupported(expr) => NormalizeResult::Unsupported(expr),
                }
            }
            TypedExprKind::TagCall {
                args: Some(args),
                variant_id,
                ..
            } => normalize_tag_call(variant_id.clone(), args, expr_id, context, substitutions),
            TypedExprKind::TagCall {
                args: None,
                variant_id,
                ..
            } => NormalizeResult::Value(ConstValue::Tag {
                name: variant_id.name,
                qual_path: Some(variant_id.union.0.as_str().to_owned()),
                args: Vec::new().into(),
            }),
            TypedExprKind::RecordValue(items) => {
                let definition = context
                    .program
                    .type_registry
                    .resolved_definition_for_type(&context.program.exprs.ty[expr_id.as_usize()]);
                let Ty::Record { fields, .. } = definition else {
                    return NormalizeResult::Unsupported(expr_id);
                };
                if items.len() != fields.len() {
                    return NormalizeResult::Unsupported(expr_id);
                }
                let mut values = Vec::with_capacity(items.len());
                for (item, field) in items.iter().zip(fields.iter()) {
                    match normalize_expr(*item, context, substitutions) {
                        NormalizeResult::Value(value) => values.push((field.0, value)),
                        NormalizeResult::Residual(_) => {
                            return NormalizeResult::Unsupported(expr_id);
                        }
                        result => return result,
                    }
                }
                NormalizeResult::Value(ConstValue::Record {
                    fields: values.into(),
                })
            }
            TypedExprKind::ResultRow(items) => {
                let arg_values = match normalize_item_values(items, expr_id, context, substitutions)
                {
                    Ok(values) => values,
                    Err(outcome) => return outcome,
                };
                NormalizeResult::Value(ConstValue::sequence(
                    arg_values,
                    match context.program.exprs.ty.get(expr_id.as_usize()) {
                        Some(Ty::Sequence(fields)) => {
                            fields.iter().map(|field| field.name).collect::<Vec<_>>()
                        }
                        _ => Vec::new(),
                    },
                ))
            }
            TypedExprKind::ArrayLit(items) => {
                let arg_values = match normalize_item_values(items, expr_id, context, substitutions)
                {
                    Ok(values) => values,
                    Err(outcome) => return outcome,
                };
                NormalizeResult::Value(ConstValue::FixedArray(arg_values.into()))
            }
            TypedExprKind::List(items) => {
                let arg_values = match normalize_item_values(items, expr_id, context, substitutions)
                {
                    Ok(values) => values,
                    Err(outcome) => return outcome,
                };
                NormalizeResult::Value(ConstValue::Sequence(arg_values.into()))
            }
            TypedExprKind::AggregateGet { base, index } => {
                normalize_aggregate_get(*base, *index, expr_id, context, substitutions)
            }
            TypedExprKind::BufGet { buf, index } => {
                normalize_buf_get(*buf, *index, expr_id, context, substitutions)
            }
            TypedExprKind::TakePtr(_) => NormalizeResult::RuntimeDependency(expr_id),
            TypedExprKind::Cast { expr, .. }
                if context.program.exprs.ty[expr_id.as_usize()].is_ptr()
                    || context.program.exprs.ty[expr.as_usize()].is_ptr() =>
            {
                NormalizeResult::RuntimeDependency(expr_id)
            }
            TypedExprKind::Ref(inner) | TypedExprKind::Deref(inner) => {
                normalize_expr(*inner, context, substitutions)
            }
            _ => NormalizeResult::Unsupported(expr_id),
        }
    })
}

fn normalize_binary(
    lhs_id: ExprId,
    rhs_id: ExprId,
    op: BinOp,
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    let lhs = normalize_expr(lhs_id, context, substitutions);
    let rhs = normalize_expr(rhs_id, context, substitutions);

    if let (NormalizeResult::RuntimeDependency(_), _) | (_, NormalizeResult::RuntimeDependency(_)) =
        (&lhs, &rhs)
    {
        return NormalizeResult::RuntimeDependency(expr_id);
    }
    if let (NormalizeResult::Unsupported(_), _) | (_, NormalizeResult::Unsupported(_)) =
        (&lhs, &rhs)
    {
        return NormalizeResult::Unsupported(expr_id);
    }

    if let (NormalizeResult::Value(lhs), NormalizeResult::Value(rhs)) = (&lhs, &rhs)
        && let Some(value) = lhs.eval_binop(&op, rhs)
    {
        return NormalizeResult::Value(value);
    }

    let lhs = match to_normal_expr(lhs) {
        Some(lhs) => lhs,
        None => return NormalizeResult::Unsupported(expr_id),
    };
    let rhs = match to_normal_expr(rhs) {
        Some(rhs) => rhs,
        None => return NormalizeResult::Unsupported(expr_id),
    };

    NormalizeResult::Residual(NormalExpr::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    })
}

fn to_normal_expr(result: NormalizeResult) -> Option<NormalExpr> {
    match result {
        NormalizeResult::Value(value) => Some(NormalExpr::Value(value)),
        NormalizeResult::Residual(expr) => Some(expr),
        NormalizeResult::RuntimeDependency(_) | NormalizeResult::Unsupported(_) => None,
    }
}

fn normalize_item_values(
    items: &[ExprId],
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> Result<Vec<ConstValue>, NormalizeResult> {
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        match normalize_expr(*item, context, substitutions) {
            NormalizeResult::RuntimeDependency(expr) => {
                return Err(NormalizeResult::RuntimeDependency(expr));
            }
            NormalizeResult::Unsupported(expr) => return Err(NormalizeResult::Unsupported(expr)),
            NormalizeResult::Value(value) => values.push(value),
            NormalizeResult::Residual(_) => return Err(NormalizeResult::Unsupported(expr_id)),
        }
    }
    Ok(values)
}

fn normalize_call(
    target: DefId,
    args: &[ExprId],
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    if has_runtime_effects(context.program, target) {
        return NormalizeResult::RuntimeDependency(expr_id);
    }

    let (normalized_args, arg_values, args_are_value_only) =
        match normalize_args(args, context, substitutions) {
            Ok(result) => result,
            Err(outcome) => return outcome,
        };

    if args_are_value_only
        && let Some(result) =
            try_evaluate_pure_call(target, &arg_values, expr_id, context, substitutions)
    {
        return result;
    }

    NormalizeResult::Residual(NormalExpr::FnCall {
        target,
        args: normalized_args,
    })
}

fn normalize_args(
    args: &[ExprId],
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> Result<(Vec<NormalExpr>, Vec<ConstValue>, bool), NormalizeResult> {
    let mut normalized_args = Vec::with_capacity(args.len());
    let mut arg_values = Vec::with_capacity(args.len());
    let mut args_are_value_only = true;

    for arg in args {
        match normalize_expr(*arg, context, substitutions) {
            NormalizeResult::RuntimeDependency(expr) => {
                return Err(NormalizeResult::RuntimeDependency(expr));
            }
            NormalizeResult::Unsupported(expr) => return Err(NormalizeResult::Unsupported(expr)),
            NormalizeResult::Value(value) => {
                normalized_args.push(NormalExpr::Value(value.clone()));
                arg_values.push(value);
            }
            NormalizeResult::Residual(expr) => {
                args_are_value_only = false;
                normalized_args.push(expr);
            }
        }
    }

    Ok((normalized_args, arg_values, args_are_value_only))
}

fn normalize_tag_call(
    variant_id: VariantId,
    args: &[ExprId],
    _expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    let (normalized_args, arg_values, _) = match normalize_args(args, context, substitutions) {
        Ok(result) => result,
        Err(outcome) => return outcome,
    };

    if args.len() == arg_values.len() {
        return NormalizeResult::Value(ConstValue::Tag {
            name: variant_id.name,
            qual_path: Some(variant_id.union.0.as_str().to_owned()),
            args: arg_values.into(),
        });
    }

    NormalizeResult::Residual(NormalExpr::TagCall {
        variant_id,
        args: normalized_args,
    })
}

fn field_from_const_value(base: &ConstValue, index: usize) -> Option<ConstValue> {
    match base {
        ConstValue::Tag { args, .. } => args.get(index).cloned(),
        ConstValue::Text(text) if index == 1 => Some(ConstValue::Int((text.len() as u64).into())),
        ConstValue::ResultRow(items)
        | ConstValue::Sequence(items)
        | ConstValue::FixedArray(items) => items.get(index).cloned(),
        ConstValue::Record { fields } => fields.get(index).map(|(_, value)| value.clone()),
        _ => None,
    }
}

fn normalize_aggregate_get(
    base_id: ExprId,
    index: usize,
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    match normalize_expr(base_id, context, substitutions) {
        NormalizeResult::Value(base) => match field_from_const_value(&base, index) {
            Some(value) => NormalizeResult::Value(value),
            None => NormalizeResult::Unsupported(expr_id),
        },
        NormalizeResult::RuntimeDependency(expr) => NormalizeResult::RuntimeDependency(expr),
        NormalizeResult::Unsupported(expr) => NormalizeResult::Unsupported(expr),
        NormalizeResult::Residual(expr) => NormalizeResult::Residual(NormalExpr::FieldGet {
            base: Box::new(expr),
            index,
        }),
    }
}

fn normalize_buf_get(
    buf_id: ExprId,
    index_id: ExprId,
    expr_id: ExprId,
    context: &mut NormalizeContext,
    substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> NormalizeResult {
    let base = normalize_expr(buf_id, context, substitutions);
    let index = normalize_expr(index_id, context, substitutions);

    if let (NormalizeResult::Value(base), NormalizeResult::Value(index)) = (&base, &index) {
        let value = match index {
            ConstValue::Int(index) => {
                ast::integer::to_usize(*index).and_then(|index| field_from_const_value(base, index))
            }
            _ => None,
        };
        return value.map_or(
            NormalizeResult::Unsupported(expr_id),
            NormalizeResult::Value,
        );
    }

    if matches!(
        (&base, &index),
        (
            NormalizeResult::RuntimeDependency(_),
            NormalizeResult::RuntimeDependency(_)
                | NormalizeResult::Value(_)
                | NormalizeResult::Residual(_)
        )
    ) {
        return match base {
            NormalizeResult::RuntimeDependency(expr) => NormalizeResult::RuntimeDependency(expr),
            _ => NormalizeResult::Unsupported(expr_id),
        };
    }

    if matches!(
        index,
        NormalizeResult::RuntimeDependency(_) | NormalizeResult::Unsupported(_)
    ) {
        return match index {
            NormalizeResult::RuntimeDependency(expr) => NormalizeResult::RuntimeDependency(expr),
            NormalizeResult::Unsupported(expr) => NormalizeResult::Unsupported(expr),
            _ => unreachable!(),
        };
    }

    NormalizeResult::Unsupported(expr_id)
}

fn try_evaluate_pure_call(
    target: DefId,
    arg_values: &[ConstValue],
    expr_id: ExprId,
    context: &mut NormalizeContext,
    _substitutions: &HashMap<DependentArgId, NormalizeResult>,
) -> Option<NormalizeResult> {
    let bind = context.program.defs.get(&target)?;
    if bind.call_capability == CallCapability::RuntimeOnly {
        return None;
    }
    let type_only_call = bind
        .param_kinds
        .iter()
        .all(|kind| matches!(kind, ParamKind::Type))
        && arg_values.is_empty();
    if bind.params.len() != arg_values.len() && !type_only_call {
        return None;
    }
    let target_context = flask::CompileTarget::Library;
    let mut evaluator = crate::eval::Evaluator::new(
        context.program,
        crate::eval::CompileTimeHost::new(&target_context),
    );
    if let Ok(value) = evaluator.call(target, arg_values) {
        return Some(NormalizeResult::Value(value));
    }

    let return_expr = match &bind.body {
        BindBody::Expr(expr) => Some(*expr),
        BindBody::Body { exprs, ret } if exprs.is_empty() => *ret,
        BindBody::Body { .. } | BindBody::Unlowered => None,
    };
    let return_expr = return_expr?;

    let frame = NormalizeCall {
        target,
        args: arg_values.to_vec(),
    };
    if !context.call_stack.insert(frame.clone()) {
        return Some(NormalizeResult::Unsupported(expr_id));
    }

    let mut folded_args = HashMap::with_capacity(arg_values.len());
    for (slot, arg) in arg_values.iter().cloned().enumerate() {
        folded_args.insert(
            DependentArgId::new(bind.dependent_binder, slot as u32),
            NormalizeResult::Value(arg),
        );
    }

    let result = normalize_expr(return_expr, context, &folded_args);
    context.call_stack.remove(&frame);

    Some(result)
}

fn has_runtime_effects(program: ResolvedProgram<'_>, target: DefId) -> bool {
    program.defs.get(&target).is_some_and(|bind| {
        bind.call_capability == CallCapability::RuntimeOnly || bind.effects.has_runtime_effects()
    })
}
#[cfg(test)]
#[path = "normalize/tests.rs"]
mod tests;
