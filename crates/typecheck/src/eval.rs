//! Execution boundary for resolved expressions.
//!
//! This is the common entry point for compile-time evaluation and the
//! interactive interpreter. It executes resolved typed expressions and keeps
//! effects behind an explicit host boundary.

use ast::{ConstValue, Literal};
use diagnostic::Diagnostic;
use diagnostic::code::{EvaluationCode, PropositionCode};
use internment::Intern;
use std::collections::HashMap;

use crate::resolved_program::ResolvedProgram;
use crate::ty::Ty;
use crate::typed::{ExprId, TypedExprKind};

mod heap;
pub use heap::{Address, EvalHeap};
mod frames;
use frames::ActiveFrame;
pub use frames::{EvalCondition, EvalFailure, EvalFrame, EvalLocal};

pub type EvalResult<T> = Result<T, Diagnostic>;

pub trait EvalHost {
    fn target(&self) -> &flask::CompileTarget;

    fn cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompileTimeHost<'a> {
    target: &'a flask::CompileTarget,
}

impl<'a> CompileTimeHost<'a> {
    pub fn new(target: &'a flask::CompileTarget) -> Self {
        Self { target }
    }
}

impl EvalHost for CompileTimeHost<'_> {
    fn target(&self) -> &flask::CompileTarget {
        self.target
    }
}

#[derive(Debug)]
pub enum EvalOutcome {
    Complete(ConstValue),
    Address(Address),
    Residual(ast::NormalExpr),
    RuntimeDependency(ExprId),
    Trap(Diagnostic),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fuel(u64);

impl Default for Fuel {
    fn default() -> Self {
        Self::new(512)
    }
}

impl Fuel {
    pub const fn new(amount: u64) -> Self {
        Self(amount)
    }

    fn consume(&mut self) -> bool {
        let available = self.0 > 0;
        self.0 = self.0.saturating_sub(1);
        available
    }
}

#[derive(Debug, Default, Clone)]
pub struct EvalState {
    pub heap: EvalHeap,
    pub fuel: Fuel,
}

impl EvalState {
    pub fn with_fuel(fuel: Fuel) -> Self {
        Self {
            heap: EvalHeap::default(),
            fuel,
        }
    }
}

#[derive(Clone)]
enum EvalValue {
    Frozen(ConstValue),
    Reference(Address),
}

impl EvalValue {
    fn freeze(self) -> EvalResult<ConstValue> {
        match self {
            Self::Frozen(value) => Ok(value),
            // compile-time-address-value
            Self::Reference(_) => Err(Diagnostic::new(
                EvaluationCode::AddressValue,
                "a virtual address cannot be used as a frozen compile-time value",
            )),
        }
    }

    fn outcome(self) -> EvalOutcome {
        match self {
            Self::Frozen(value) => EvalOutcome::Complete(value),
            Self::Reference(address) => EvalOutcome::Address(address),
        }
    }
}

#[derive(Clone, Copy)]
enum LocalValue {
    Stored(Address),
    Reference(Address),
}

pub struct Evaluator<'p, H> {
    program: ResolvedProgram<'p>,
    programs: Vec<ResolvedProgram<'p>>,
    host: H,
    fuel: Fuel,
    heap: EvalHeap,
    locals: Vec<HashMap<Intern<String>, LocalValue>>,
    returned: Option<EvalValue>,
    when_subjects: HashMap<ExprId, EvalValue>,
    frames: Vec<ActiveFrame>,
    failure_frames: Option<Vec<EvalFrame>>,
}

impl<'p, H: EvalHost> Evaluator<'p, H> {
    pub fn new(program: ResolvedProgram<'p>, host: H) -> Self {
        Self {
            program,
            programs: Vec::new(),
            host,
            fuel: Fuel::new(512),
            heap: EvalHeap::default(),
            locals: Vec::new(),
            returned: None,
            when_subjects: HashMap::new(),
            frames: Vec::new(),
            failure_frames: None,
        }
    }

    pub fn with_fuel(mut self, fuel: Fuel) -> Self {
        self.fuel = fuel;
        self
    }

    pub fn with_programs(
        mut self,
        programs: impl IntoIterator<Item = ResolvedProgram<'p>>,
    ) -> Self {
        self.programs = programs.into_iter().collect();
        self
    }

    pub fn eval(&mut self, expr: ExprId) -> EvalResult<EvalOutcome> {
        let root = self.frames.is_empty();
        if root {
            self.failure_frames = None;
            self.frames
                .push(ActiveFrame::expression(self.program.file_id));
        }
        let previous = self
            .frames
            .last_mut()
            .expect("active evaluation frame")
            .expression
            .replace(expr);
        let result = self.eval_inner(expr);
        let result = self
            .returned
            .as_ref()
            .map(|value| Ok(value.clone().outcome()))
            .unwrap_or(result);
        if !matches!(
            result,
            Ok(EvalOutcome::Complete(_) | EvalOutcome::Address(_))
        ) {
            self.capture_frames();
        }
        self.frames
            .last_mut()
            .expect("active evaluation frame")
            .expression = previous;
        if root {
            self.frames.pop();
            if matches!(
                result,
                Ok(EvalOutcome::Complete(_) | EvalOutcome::Address(_))
            ) {
                self.failure_frames = None;
            }
        }
        result
    }

    fn eval_inner(&mut self, expr: ExprId) -> EvalResult<EvalOutcome> {
        if let Some(value) = &self.returned {
            return Ok(value.clone().outcome());
        }
        if self.host.cancelled() {
            return Ok(EvalOutcome::Cancelled);
        }
        if !self.fuel.consume() {
            // compile-time-fuel-exhausted
            return Err(Diagnostic::new(
                EvaluationCode::FuelExhausted,
                "compile-time evaluation exceeded its fuel limit",
            ));
        }

        if self.program.is_local_callable(expr) {
            return Ok(EvalOutcome::Complete(ConstValue::Record {
                fields: Vec::new().into(),
            }));
        }
        if let Some(definition) = self.program.local_call(expr) {
            return self.eval_local_call(expr, definition);
        }
        if let Some(value) = self.when_subjects.get(&expr) {
            return Ok(value.clone().outcome());
        }

        let Some(kind) = self.program.exprs.kind_of(expr) else {
            // compile-time-invalid-expression
            return Err(Diagnostic::new(
                EvaluationCode::InvalidExpression,
                "expression is not present in the resolved program",
            ));
        };

        let direct = match kind {
            TypedExprKind::Domain(domain) => Some(ConstValue::Domain(domain.clone().into())),
            TypedExprKind::Invalid => None,
            TypedExprKind::Todo => {
                // compile-time-unfilled-hole
                return Ok(EvalOutcome::Trap(Diagnostic::new(
                    EvaluationCode::UnfilledHole,
                    "evaluation reached an unfilled #todo hole",
                )));
            }
            TypedExprKind::Lit(Literal::Int(value)) => Some(ConstValue::Int(*value)),
            TypedExprKind::Lit(Literal::Float(value)) => Some(ConstValue::Float(*value)),
            TypedExprKind::Lit(Literal::String(value)) => Some(ConstValue::Text(value.clone())),
            TypedExprKind::Negate(inner) => match self.complete(*inner)? {
                ConstValue::Int(value) => Some(ConstValue::Int(-value)),
                _ => None,
            },
            TypedExprKind::Binary { op, lhs, rhs } => {
                let lhs = self.complete(*lhs)?;
                let rhs = self.complete(*rhs)?;
                lhs.eval_binop(op, &rhs)
            }
            TypedExprKind::InvalidOperator { error, .. } => {
                // compile-time-invalid-operator
                return Ok(EvalOutcome::Trap(Diagnostic::new(
                    EvaluationCode::InvalidOperator,
                    format!("cannot execute an unresolved operator: {error:?}"),
                )));
            }
            TypedExprKind::IntrinsicCall { op, args } => {
                if matches!(
                    op,
                    crate::intrinsic::IntrinsicOp::AddressLoad
                        | crate::intrinsic::IntrinsicOp::AddressStore
                        | crate::intrinsic::IntrinsicOp::AddressOffset
                ) {
                    let Some(address_expr) = args.first() else {
                        // compile-time-invalid-intrinsic
                        return Err(Diagnostic::new(
                            EvaluationCode::InvalidIntrinsic,
                            "address intrinsic is missing its address operand",
                        ));
                    };
                    let address = match self.eval(*address_expr)? {
                        EvalOutcome::Address(address) => address,
                        bubble @ (EvalOutcome::RuntimeDependency(_)
                        | EvalOutcome::Residual(_)
                        | EvalOutcome::Cancelled
                        | EvalOutcome::Trap(_)) => return Ok(bubble),
                        EvalOutcome::Complete(_) => {
                            // compile-time-invalid-address
                            return Err(Diagnostic::new(
                                EvaluationCode::InvalidAddress,
                                "address intrinsic requires a virtual address",
                            ));
                        }
                    };
                    return Ok(EvalOutcome::Complete(match op {
                        crate::intrinsic::IntrinsicOp::AddressLoad => self.heap.freeze(address)?,
                        crate::intrinsic::IntrinsicOp::AddressStore => {
                            let Some(value_expr) = args.get(1) else {
                                // compile-time-invalid-intrinsic
                                return Err(Diagnostic::new(
                                    EvaluationCode::InvalidIntrinsic,
                                    "address store is missing its value operand",
                                ));
                            };
                            let value = self.complete(*value_expr)?;
                            if let Some(value) = &self.returned {
                                return Ok(value.clone().outcome());
                            }
                            self.heap.store(address, value)?;
                            ConstValue::Record {
                                fields: Vec::new().into(),
                            }
                        }
                        crate::intrinsic::IntrinsicOp::AddressOffset => {
                            let Some(offset_expr) = args.get(1) else {
                                // compile-time-invalid-intrinsic
                                return Err(Diagnostic::new(
                                    EvaluationCode::InvalidIntrinsic,
                                    "address offset is missing its offset operand",
                                ));
                            };
                            let offset = self
                                .complete(*offset_expr)?
                                .as_const_size_int()
                                .ok_or_else(|| {
                                    // compile-time-invalid-address-offset
                                    Diagnostic::new(
                                        EvaluationCode::InvalidAddressOffset,
                                        "virtual address offset is not an integer",
                                    )
                                })?;
                            let offset = isize::try_from(offset).map_err(|_| {
                                // compile-time-invalid-address-offset
                                Diagnostic::new(
                                    EvaluationCode::InvalidAddressOffset,
                                    "virtual address offset is outside the supported range",
                                )
                            })?;
                            return Ok(EvalOutcome::Address(address.offset(offset)?));
                        }
                        _ => unreachable!(),
                    }));
                } else if matches!(
                    op,
                    crate::intrinsic::IntrinsicOp::AddressToBits
                        | crate::intrinsic::IntrinsicOp::BitsToAddress
                ) {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                }
                let values = args
                    .iter()
                    .map(|arg| self.complete(*arg))
                    .collect::<EvalResult<Vec<_>>>()?;
                if let Some(value) = op.fold_comparison(&values) {
                    return Ok(EvalOutcome::Complete(value));
                }
                if let [left, right] = args.as_slice()
                    && let Some(value) = crate::anonymous_arithmetic::fold(
                        *op,
                        &values,
                        [
                            &self.program.exprs.ty[left.as_usize()],
                            &self.program.exprs.ty[right.as_usize()],
                        ],
                    )
                {
                    return value.map(EvalOutcome::Complete);
                }
                let source_width = args.first().and_then(|arg| {
                    self.program
                        .type_registry
                        .integer_width_for_type(&self.program.exprs.ty[arg.as_usize()])
                });
                let result_width = self
                    .program
                    .type_registry
                    .integer_width_for_type(&self.program.exprs.ty[expr.as_usize()])
                    .or_else(|| {
                        matches!(
                            self.program.exprs.ty[expr.as_usize()],
                            ast::Ty::Proposition(_)
                        )
                        .then_some(1)
                    });
                let (Some(source_width), Some(result_width)) = (source_width, result_width) else {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                };
                Some(
                    op.fold(&values, source_width, result_width)
                        .map_err(|error| op.diagnostic(&error))?
                        .ok_or_else(|| {
                            // compile-time-unsupported-intrinsic
                            Diagnostic::new(
                                EvaluationCode::UnsupportedIntrinsic,
                                "intrinsic is not supported by the compile-time evaluator",
                            )
                        })?,
                )
            }
            TypedExprKind::UnionPayload { expr: inner, .. } => {
                let ConstValue::Tag { args, .. } = self.complete(*inner)? else {
                    // compile-time-invalid-union-payload
                    return Err(Diagnostic::new(
                        EvaluationCode::InvalidUnionPayload,
                        "expected a tagged union value",
                    ));
                };
                args.first().cloned()
            }
            TypedExprKind::Cast { expr: inner, ty } => {
                let value = self.complete(*inner)?;
                let source_ty = &self.program.exprs.ty[inner.as_usize()];
                if let Some(value) = crate::discriminant::cast_constant(
                    self.program.type_registry,
                    source_ty,
                    ty,
                    &value,
                ) {
                    return Ok(EvalOutcome::Complete(value));
                }
                let source_width = self.program.type_registry.integer_width_for_type(source_ty);
                let target_width = self.program.type_registry.integer_width_for_type(ty);
                let (ConstValue::Int(_), Some(source_width), Some(target_width)) =
                    (&value, source_width, target_width)
                else {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                };
                if source_width == target_width {
                    Some(value)
                } else {
                    let op = if target_width < source_width {
                        crate::intrinsic::IntrinsicOp::BitsTruncate
                    } else if value.as_const_size_int().is_some_and(|value| value < 0) {
                        crate::intrinsic::IntrinsicOp::BitsExtendSigned
                    } else {
                        crate::intrinsic::IntrinsicOp::BitsExtendUnsigned
                    };
                    Some(
                        op.fold(&[value], source_width, target_width)
                            .map_err(|error| op.diagnostic(&error))?
                            .ok_or_else(|| {
                                // compile-time-unsupported-cast
                                Diagnostic::new(
                                    EvaluationCode::UnsupportedCast,
                                    "integer cast is not supported by the compile-time evaluator",
                                )
                            })?,
                    )
                }
            }
            TypedExprKind::FnCall { target, args, .. } => {
                if args.is_none()
                    && let Some(local) = self.local(target.0)
                {
                    return Ok(self.read_local(local)?.outcome());
                }
                let args = args
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|item| self.value(*item))
                    .collect::<EvalResult<Vec<_>>>()?;
                if let Some(value) = &self.returned {
                    return Ok(value.clone().outcome());
                }
                return Ok(self.call_values(*target, &args)?.outcome());
            }
            TypedExprKind::Bind {
                name,
                stmts,
                body,
                unassigned,
            } => {
                if *unassigned {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                }
                self.locals.push(HashMap::new());
                let value = self.eval_returning_sequence(stmts, Some(*body));
                self.locals.pop();
                let value = value?;
                let local = self.store_local(value.clone());
                if let Some(frame) = self.locals.last_mut() {
                    frame.insert(*name, local);
                }
                return Ok(value.outcome());
            }
            TypedExprKind::Reassign {
                name,
                value,
                operator,
            } => {
                let local = self.local(*name).ok_or_else(|| {
                    // compile-time-unassigned-value
                    Diagnostic::new(
                        EvaluationCode::UnassignedValue,
                        "compile-time reassignment refers to an unassigned value",
                    )
                })?;
                let replacement = self.value(*value)?;
                if let Some(value) = &self.returned {
                    return Ok(value.clone().outcome());
                }
                let replacement = if let Some(operator) = operator {
                    let current = self.read_local(local)?.freeze()?;
                    EvalValue::Frozen(
                        current
                            .eval_binop(operator, &replacement.freeze()?)
                            .ok_or_else(|| {
                                // compile-time-invalid-reassignment
                                Diagnostic::new(
                                    EvaluationCode::InvalidReassignment,
                                    "compile-time reassignment operator cannot be evaluated",
                                )
                            })?,
                    )
                } else {
                    replacement
                };
                let local = match (local, &replacement) {
                    (LocalValue::Stored(address), EvalValue::Frozen(value)) => {
                        self.heap.store(address, value.clone())?;
                        LocalValue::Stored(address)
                    }
                    (LocalValue::Reference(address), EvalValue::Frozen(value)) => {
                        self.heap.store(address, value.clone())?;
                        LocalValue::Reference(address)
                    }
                    _ => self.store_local(replacement.clone()),
                };
                self.locals
                    .iter_mut()
                    .rev()
                    .find(|frame| frame.contains_key(name))
                    .unwrap()
                    .insert(*name, local);
                return Ok(replacement.outcome());
            }
            TypedExprKind::Destructure {
                tag_name,
                value,
                field_bindings,
                ..
            } => {
                let subject = self.complete(*value)?;
                let fields = match &self.program.exprs.ty[value.as_usize()] {
                    crate::ty::Ty::Record { fields, .. } => {
                        Some(fields.iter().map(|(name, _)| *name).collect::<Vec<_>>())
                    }
                    crate::ty::Ty::Union { variants, .. } => variants
                        .iter()
                        .find(|variant| variant.name == *tag_name)
                        .map(|variant| {
                            variant
                                .fields
                                .iter()
                                .map(|(name, _)| *name)
                                .collect::<Vec<_>>()
                        }),
                    _ => None,
                };
                let values = match &subject {
                    ConstValue::Record { fields } => {
                        fields.iter().map(|(_, value)| value).collect::<Vec<_>>()
                    }
                    ConstValue::Tag { name, args, .. } if name == tag_name => args.iter().collect(),
                    _ => {
                        // compile-time-invalid-destructure
                        return Err(Diagnostic::new(
                            EvaluationCode::InvalidDestructure,
                            "compile-time destructuring requires a matching record or tag value",
                        ));
                    }
                };
                let Some(fields) = fields else {
                    // compile-time-invalid-destructure
                    return Err(Diagnostic::new(
                        EvaluationCode::InvalidDestructure,
                        "compile-time destructuring type is not a record or union",
                    ));
                };
                let Some(locals) = self.locals.last_mut() else {
                    // compile-time-invalid-destructure
                    return Err(Diagnostic::new(
                        EvaluationCode::InvalidDestructure,
                        "compile-time destructuring has no active local scope",
                    ));
                };
                for (field_name, binding_name) in field_bindings {
                    let Some(index) = fields.iter().position(|name| name == field_name) else {
                        // compile-time-unknown-field
                        return Err(Diagnostic::new(
                            EvaluationCode::UnknownField,
                            "compile-time destructuring field does not exist",
                        ));
                    };
                    let Some(value) = values.get(index) else {
                        // compile-time-invalid-destructure
                        return Err(Diagnostic::new(
                            EvaluationCode::InvalidDestructure,
                            "compile-time destructuring value has too few fields",
                        ));
                    };
                    locals.insert(
                        *binding_name,
                        LocalValue::Stored(self.heap.allocate((*value).clone())),
                    );
                }
                Some(subject)
            }
            TypedExprKind::RecordValue(items) => {
                if let Some(address) = self.reference_aggregate_value(expr, items)? {
                    return Ok(EvalOutcome::Address(address));
                }
                let values = items
                    .iter()
                    .map(|item| self.complete(*item))
                    .collect::<EvalResult<Vec<_>>>()?;
                let names = match self.program.exprs.ty_of(expr) {
                    Some(Ty::Record { fields, .. }) => {
                        fields.iter().map(|(name, _)| *name).collect::<Vec<_>>()
                    }
                    _ => (0..values.len())
                        .map(|index| Intern::new(index.to_string()))
                        .collect(),
                };
                Some(ConstValue::Record {
                    fields: names.into_iter().zip(values).collect::<Vec<_>>().into(),
                })
            }
            TypedExprKind::ResultRow(items) => {
                let values = items
                    .iter()
                    .map(|item| self.complete(*item))
                    .collect::<EvalResult<Vec<_>>>()?;
                let names = match self.program.exprs.ty.get(expr.as_usize()) {
                    Some(Ty::Sequence(fields)) => {
                        fields.iter().map(|field| field.name).collect::<Vec<_>>()
                    }
                    _ => Vec::new(),
                };
                Some(ConstValue::sequence(values, names))
            }
            TypedExprKind::ArrayLit(items) => Some(ConstValue::FixedArray(
                items
                    .iter()
                    .map(|item| self.complete(*item))
                    .collect::<EvalResult<Vec<_>>>()?
                    .into(),
            )),
            TypedExprKind::List(items) => Some(ConstValue::Sequence(
                items
                    .iter()
                    .map(|item| self.complete(*item))
                    .collect::<EvalResult<Vec<_>>>()?
                    .into(),
            )),
            TypedExprKind::TagCall {
                variant_id, args, ..
            } => {
                if let Some(address) =
                    self.reference_aggregate_value(expr, args.as_deref().unwrap_or_default())?
                {
                    return Ok(EvalOutcome::Address(address));
                }
                Some(ConstValue::Tag {
                    name: variant_id.name,
                    qual_path: Some(variant_id.union.0.as_str().to_owned()),
                    args: args
                        .as_ref()
                        .map(|items| {
                            items
                                .iter()
                                .map(|item| self.complete(*item))
                                .collect::<EvalResult<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default()
                        .into(),
                })
            }
            TypedExprKind::Rematerialize { constant, .. } => Some(constant.clone()),
            TypedExprKind::TakePtr(inner) | TypedExprKind::Ref(inner) => {
                return Ok(EvalOutcome::Address(self.place(*inner)?));
            }
            TypedExprKind::FixedArrayAlloc { init, size } => {
                let value = self.complete(*init)?;
                let Some(count) = size.as_const_int() else {
                    return Ok(EvalOutcome::Residual(size.clone()));
                };
                let count = usize::try_from(count).map_err(|_| {
                    // compile-time-invalid-allocation
                    Diagnostic::new(
                        EvaluationCode::InvalidAllocation,
                        "fixed-array allocation size is invalid",
                    )
                })?;
                let mut values = Vec::new();
                values.try_reserve_exact(count).map_err(|error| {
                    // compile-time-invalid-allocation
                    Diagnostic::new(
                        EvaluationCode::InvalidAllocation,
                        format!("fixed-array allocation failed: {error}"),
                    )
                })?;
                values.resize(count, value);
                Some(ConstValue::FixedArray(values.into()))
            }
            TypedExprKind::Deref(inner) => {
                let address = match self.eval(*inner)? {
                    EvalOutcome::Address(address) => address,
                    bubble @ (EvalOutcome::RuntimeDependency(_)
                    | EvalOutcome::Residual(_)
                    | EvalOutcome::Cancelled
                    | EvalOutcome::Trap(_)) => return Ok(bubble),
                    EvalOutcome::Complete(_) => {
                        // compile-time-invalid-deref
                        return Err(Diagnostic::new(
                            EvaluationCode::InvalidDeref,
                            "cannot dereference a non-address compile-time value",
                        ));
                    }
                };
                if self
                    .program
                    .type_registry
                    .contains_references(&self.program.exprs.ty[expr.index()])
                {
                    return Ok(EvalOutcome::Address(address));
                }
                Some(self.heap.freeze(address)?)
            }
            TypedExprKind::AggregateGet { base, index } => match self.eval(*base)? {
                EvalOutcome::Address(address) => {
                    let field = self.heap.project(address, *index)?;
                    if let Some(reference) = self.heap.reference(field) {
                        return Ok(EvalOutcome::Address(reference));
                    }
                    Some(self.heap.freeze(field)?)
                }
                EvalOutcome::Complete(ConstValue::ResultRow(items)) => items.get(*index).cloned(),
                EvalOutcome::Complete(ConstValue::Tag { args, .. }) => args.get(*index).cloned(),
                EvalOutcome::Complete(ConstValue::Record { fields }) => {
                    fields.get(*index).map(|(_, value)| value.clone())
                }
                EvalOutcome::Complete(_) => None,
                bubble @ (EvalOutcome::RuntimeDependency(_)
                | EvalOutcome::Residual(_)
                | EvalOutcome::Cancelled
                | EvalOutcome::Trap(_)) => return Ok(bubble),
            },
            TypedExprKind::SliceGet { .. } => return Ok(EvalOutcome::Address(self.place(expr)?)),
            TypedExprKind::BufGet { buf, index } => {
                let index = self.complete(*index)?.as_const_size_int().ok_or_else(|| {
                    // compile-time-invalid-index
                    Diagnostic::new(
                        EvaluationCode::InvalidIndex,
                        "buffer index is not an integer",
                    )
                })?;
                let index = usize::try_from(index).map_err(|_| {
                    // compile-time-invalid-index
                    Diagnostic::new(EvaluationCode::InvalidIndex, "buffer index is negative")
                })?;
                match self.eval(*buf)? {
                    EvalOutcome::Address(address) => {
                        let element = self.index_address(*buf, address, index)?;
                        Some(self.heap.freeze(element)?)
                    }
                    EvalOutcome::Complete(
                        ConstValue::FixedArray(items) | ConstValue::Sequence(items),
                    ) => Some(items.get(index).cloned().ok_or_else(|| {
                        // compile-time-index-out-of-bounds
                        Diagnostic::new(
                            EvaluationCode::IndexOutOfBounds,
                            "buffer index is outside the initialized aggregate",
                        )
                    })?),
                    EvalOutcome::Complete(ConstValue::Text(text)) => text
                        .as_bytes()
                        .get(index)
                        .map(|byte| ConstValue::Int((*byte as u128).into())),
                    EvalOutcome::Complete(_) => None,
                    bubble @ (EvalOutcome::RuntimeDependency(_)
                    | EvalOutcome::Residual(_)
                    | EvalOutcome::Cancelled
                    | EvalOutcome::Trap(_)) => return Ok(bubble),
                }
            }
            TypedExprKind::Block { statements, value } => {
                self.locals.push(HashMap::new());
                let result = self.eval_returning_sequence(statements, *value);
                self.locals.pop();
                return result.map(|value| value.outcome());
            }
            TypedExprKind::Return { value, .. } => {
                let result = self.eval_returning_sequence(&[], *value)?;
                self.returned = Some(result.clone());
                return Ok(result.outcome());
            }
            TypedExprKind::If(if_expr) => {
                if self.condition_matches(&if_expr.condition)? {
                    let value = self.eval_returning_sequence(&if_expr.stmts, if_expr.ret)?;
                    self.returned = Some(value.clone());
                    return Ok(value.outcome());
                }
                Some(ConstValue::Record {
                    fields: Vec::new().into(),
                })
            }
            TypedExprKind::When(when_expr) => {
                let subject = when_expr.subject.map(|id| self.value(id)).transpose()?;
                if let Some(value) = &self.returned {
                    return Ok(value.clone().outcome());
                }
                let previous = when_expr
                    .subject
                    .zip(subject.clone())
                    .and_then(|(id, value)| self.when_subjects.insert(id, value));
                let result = (|| {
                    for arm in &when_expr.arms {
                        self.locals.push(HashMap::new());
                        let result = (|| {
                            let (matches, body) = match arm {
                                crate::typed::TypedWhenArm::Cond {
                                    condition, body, ..
                                } => (self.condition_matches(condition)?, *body),
                                crate::typed::TypedWhenArm::Is { pattern, body, .. } => {
                                    let value = subject.clone().unwrap().freeze()?;
                                    (self.match_and_bind(&pattern.value, &value)?, *body)
                                }
                                crate::typed::TypedWhenArm::Else(body, _) => (true, *body),
                            };
                            if let Some(value) = &self.returned {
                                Ok(Some(value.clone()))
                            } else if matches {
                                Ok(Some(self.value(body)?))
                            } else {
                                Ok(None)
                            }
                        })();
                        self.locals.pop();
                        if let Some(value) = result? {
                            return Ok(value);
                        }
                    }
                    Ok(EvalValue::Frozen(ConstValue::Record {
                        fields: Vec::new().into(),
                    }))
                })();
                if let Some(id) = when_expr.subject {
                    self.when_subjects.remove(&id);
                    if let Some(previous) = previous {
                        self.when_subjects.insert(id, previous);
                    }
                }
                return result.map(EvalValue::outcome);
            }
            TypedExprKind::Loop(loop_expr) => {
                match &loop_expr.kind {
                    crate::typed::TypedLoopKind::While {
                        condition, stmts, ..
                    } => {
                        while self.condition_matches(condition)? {
                            for stmt in stmts {
                                self.evaluated_value(*stmt)?;
                                if let Some(value) = &self.returned {
                                    return Ok(value.clone().outcome());
                                }
                            }
                        }
                    }
                    crate::typed::TypedLoopKind::ForIn {
                        iterable,
                        traversal,
                    } => match traversal {
                        crate::typed::SequenceTraversal::Fixed { iterations } => {
                            self.evaluated_value(*iterable)?;
                            for statements in iterations {
                                self.eval_scoped_iteration(statements)?;
                                if let Some(value) = &self.returned {
                                    return Ok(value.clone().outcome());
                                }
                            }
                        }
                        crate::typed::SequenceTraversal::RuntimeView {
                            index,
                            length,
                            stmts,
                        } => {
                            self.evaluated_value(*iterable)?;
                            let count =
                                self.complete(*length)?.as_const_size_int().ok_or_else(|| {
                                    // compile-time-invalid-sequence-length
                                    Diagnostic::new(
                                        EvaluationCode::InvalidSequenceLength,
                                        "sequence length is not an integer",
                                    )
                                })?;
                            for current in 0..count {
                                let address = self.heap.allocate(ConstValue::Int(current.into()));
                                self.locals
                                    .last_mut()
                                    .unwrap()
                                    .insert(*index, LocalValue::Stored(address));
                                self.eval_scoped_iteration(stmts)?;
                                if let Some(value) = &self.returned {
                                    return Ok(value.clone().outcome());
                                }
                            }
                            self.locals.last_mut().unwrap().remove(index);
                        }
                        crate::typed::SequenceTraversal::IntegerRange {
                            variable,
                            start,
                            end,
                            stmts,
                        } => {
                            let start = self.complete(*start)?;
                            let end = self.complete(*end)?;
                            let (ConstValue::Int(mut current), ConstValue::Int(end)) = (start, end)
                            else {
                                return Ok(EvalOutcome::RuntimeDependency(expr));
                            };
                            let previous = self.local(*variable);
                            let step = i256::I256::from(if current > end { -1 } else { 1 });
                            loop {
                                let value = LocalValue::Stored(
                                    self.heap.allocate(ConstValue::Int(current)),
                                );
                                let Some(locals) = self.locals.last_mut() else {
                                    // compile-time-invalid-loop
                                    return Err(Diagnostic::new(
                                        EvaluationCode::InvalidLoop,
                                        "for-loop has no active local scope",
                                    ));
                                };
                                locals.insert(*variable, value);
                                self.eval_scoped_iteration(stmts)?;
                                if let Some(value) = &self.returned {
                                    return Ok(value.clone().outcome());
                                }
                                if current == end {
                                    break;
                                }
                                current = current.checked_add(step).ok_or_else(|| {
                                    // compile-time-range-overflow
                                    Diagnostic::new(
                                        EvaluationCode::RangeOverflow,
                                        "integer range overflowed its bounds",
                                    )
                                })?;
                            }
                            if let Some(locals) = self.locals.last_mut() {
                                if let Some(previous) = previous {
                                    locals.insert(*variable, previous);
                                } else {
                                    locals.remove(variable);
                                }
                            }
                        }
                        crate::typed::SequenceTraversal::Unresolved => {
                            return Ok(EvalOutcome::RuntimeDependency(expr));
                        }
                    },
                }
                Some(ConstValue::Record {
                    fields: Vec::new().into(),
                })
            }
            TypedExprKind::Range { start, end } => {
                let start = self.complete(*start)?;
                let end = self.complete(*end)?;
                let (ConstValue::Int(mut current), ConstValue::Int(end)) = (start, end) else {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                };
                let mut values = Vec::new();
                let step = i256::I256::from(if current > end { -1 } else { 1 });
                loop {
                    if !self.fuel.consume() {
                        // compile-time-fuel-exhausted
                        return Err(Diagnostic::new(
                            EvaluationCode::FuelExhausted,
                            "compile-time range evaluation exceeded its fuel limit",
                        ));
                    }
                    values.push(ConstValue::Int(current));
                    if current == end {
                        break;
                    }
                    current = current.checked_add(step).ok_or_else(|| {
                        // compile-time-range-overflow
                        Diagnostic::new(
                            EvaluationCode::RangeOverflow,
                            "compile-time range overflowed its integer bounds",
                        )
                    })?;
                }
                Some(ConstValue::Sequence(values.into()))
            }
            TypedExprKind::AggregateSet {
                base,
                index,
                value,
                operator,
            } => {
                let base = self.place(*base)?;
                let address = self.heap.project(base, *index)?;
                Some(self.replace_at(address, *value, operator.as_ref())?)
            }
            TypedExprKind::BufSet {
                buf,
                index,
                value,
                operator,
            } => {
                let base = self.place(*buf)?;
                let index = self.index(*index)?;
                let address = self.index_address(*buf, base, index)?;
                Some(self.replace_at(address, *value, operator.as_ref())?)
            }
            TypedExprKind::RecordSet {
                base,
                field,
                value,
                operator,
            } => {
                let ty = self.program.exprs.ty[base.as_usize()].clone();
                let ty =
                    crate::ty::reference_pointee_for_type(&ty, Some(self.program.type_registry))
                        .unwrap_or(ty);
                let definition = self.program.type_registry.resolved_definition_for_type(&ty);
                let index = definition
                    .named_field(*field)
                    .map(|(index, _)| index)
                    .ok_or_else(|| {
                        // compile-time-unknown-field
                        Diagnostic::new(
                            EvaluationCode::UnknownField,
                            "record store field does not exist",
                        )
                    })?;
                let base = self.place(*base)?;
                let address = self.heap.project(base, index)?;
                Some(self.replace_at(address, *value, operator.as_ref())?)
            }
            TypedExprKind::Target => match self.host.target() {
                flask::CompileTarget::Concrete(target) => Some(ConstValue::Record {
                    fields: vec![
                        (
                            internment::Intern::from_ref("arch"),
                            ConstValue::Text(target.arch_literal().into_owned().into()),
                        ),
                        (
                            internment::Intern::from_ref("vendor"),
                            ConstValue::Text(target.vendor_literal().into()),
                        ),
                        (
                            internment::Intern::from_ref("os"),
                            ConstValue::Text(target.os_literal().into_owned().into()),
                        ),
                        (
                            internment::Intern::from_ref("triple"),
                            ConstValue::Text(target.raw.clone()),
                        ),
                    ]
                    .into(),
                }),
                flask::CompileTarget::Library => None,
            },
            TypedExprKind::TargetField(field) => match self.host.target() {
                flask::CompileTarget::Concrete(target) => match field.as_str() {
                    "arch" => Some(ConstValue::Text(target.arch_literal().into_owned().into())),
                    "vendor" => Some(ConstValue::Text(target.vendor_literal().into())),
                    "os" => Some(ConstValue::Text(target.os_literal().into_owned().into())),
                    "triple" => Some(ConstValue::Text(target.raw.clone())),
                    _ => None,
                },
                flask::CompileTarget::Library => None,
            },
            TypedExprKind::TargetQuery { kind, operand } => {
                let layout = self.program.target_layout.as_ref().ok_or_else(|| {
                    // compile-time-target-unavailable
                    Diagnostic::new(
                        EvaluationCode::TargetUnavailable,
                        "target queries require a concrete compile target",
                    )
                })?;
                Some(ConstValue::Int(i256::I256::from(
                    layout
                        .query(*kind, operand, Some(self.program.type_registry))
                        .map_err(|_| {
                            // compile-time-target-query-failed
                            Diagnostic::new(
                                EvaluationCode::TargetQueryFailed,
                                "target layout could not answer the compile-time query",
                            )
                        })?,
                )))
            }
            TypedExprKind::SelfRef { target } => {
                return match self.local(target.0) {
                    Some(local) => Ok(self.read_local(local)?.outcome()),
                    None => Ok(EvalOutcome::RuntimeDependency(expr)),
                };
            }
            TypedExprKind::Emit { .. } => {
                return Ok(EvalOutcome::RuntimeDependency(expr));
            }
        };

        if let Some(mut value) = direct {
            if matches!(
                kind,
                TypedExprKind::IntrinsicCall { .. } | TypedExprKind::Cast { .. }
            ) && let ConstValue::Int(bits) = &mut value
                && self
                    .program
                    .type_registry
                    .integer_operation_interpretation_for_type(
                        &self.program.exprs.ty[expr.as_usize()],
                    )
                    == Some(ast::ty::IntegerInterpretation::Signed)
                && let Some(width) = self
                    .program
                    .type_registry
                    .integer_width_for_type(&self.program.exprs.ty[expr.as_usize()])
                && width < i256::I256::BITS
            {
                let modulus = i256::I256::from(1) << width;
                if *bits >= (modulus >> 1) {
                    *bits -= modulus;
                }
            }
            return Ok(EvalOutcome::Complete(value));
        }

        match self.program.exprs.const_value_of(expr) {
            Some(Some(value)) => Ok(EvalOutcome::Complete(value.clone())),
            // compile-time-unresolved-expression
            Some(None) if matches!(kind, TypedExprKind::Bind { .. }) => Err(Diagnostic::new(
                EvaluationCode::UnresolvedExpression,
                "typed expression could not be evaluated",
            )),
            Some(None) => Ok(EvalOutcome::RuntimeDependency(expr)),
            None => unreachable!("expression was checked above"),
        }
    }

    pub(crate) fn eval_after(
        &mut self,
        prefix: &[ExprId],
        expr: ExprId,
    ) -> EvalResult<EvalOutcome> {
        self.locals.push(HashMap::new());
        let result = (|| {
            for statement in prefix {
                match self.eval(*statement)? {
                    EvalOutcome::Complete(_) | EvalOutcome::Address(_) => {}
                    outcome => return Ok(outcome),
                }
            }
            self.eval(expr)
        })();
        self.locals.pop();
        result
    }

    fn local(&self, name: Intern<String>) -> Option<LocalValue> {
        self.locals
            .iter()
            .rev()
            .find_map(|frame| frame.get(&name))
            .copied()
    }

    fn read_local(&self, local: LocalValue) -> EvalResult<EvalValue> {
        match local {
            LocalValue::Stored(address) => self.heap.freeze(address).map(EvalValue::Frozen),
            LocalValue::Reference(address) => Ok(EvalValue::Reference(address)),
        }
    }

    fn store_local(&mut self, value: EvalValue) -> LocalValue {
        match value {
            EvalValue::Frozen(value) => LocalValue::Stored(self.heap.allocate(value)),
            EvalValue::Reference(address) => LocalValue::Reference(address),
        }
    }

    fn eval_scoped_iteration(&mut self, statements: &[ExprId]) -> EvalResult<()> {
        if !self.fuel.consume() {
            // compile-time-fuel-exhausted
            return Err(Diagnostic::new(
                EvaluationCode::FuelExhausted,
                "sequence traversal exceeded its fuel limit",
            ));
        }
        let names = statements
            .iter()
            .filter_map(|statement| match self.program.exprs.kind_of(*statement) {
                Some(TypedExprKind::Bind { name, .. }) => Some(*name),
                _ => None,
            })
            .collect::<Vec<_>>();
        let saved = self
            .locals
            .last()
            .map(|locals| {
                names
                    .iter()
                    .map(|name| (*name, locals.get(name).cloned()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let result = (|| {
            for statement in statements {
                self.evaluated_value(*statement)?;
                if self.returned.is_some() {
                    break;
                }
            }
            Ok(())
        })();
        if let Some(locals) = self.locals.last_mut() {
            for (name, value) in saved {
                if let Some(value) = value {
                    locals.insert(name, value);
                } else {
                    locals.remove(&name);
                }
            }
        }
        result
    }

    fn complete(&mut self, expr: ExprId) -> EvalResult<ConstValue> {
        self.value(expr)?.freeze()
    }

    fn value(&mut self, expr: ExprId) -> EvalResult<EvalValue> {
        if let Some(TypedExprKind::Bind { name, .. }) = self.program.exprs.kind_of(expr)
            && let Some(local) = self.local(*name)
        {
            return self.read_local(local);
        }
        self.evaluated_value(expr)
    }

    fn evaluated_value(&mut self, expr: ExprId) -> EvalResult<EvalValue> {
        match self.eval(expr)? {
            EvalOutcome::Complete(value) => Ok(EvalValue::Frozen(value)),
            EvalOutcome::Address(address) => Ok(EvalValue::Reference(address)),
            // compile-time-residual-value
            EvalOutcome::Residual(_) => Err(Diagnostic::new(
                EvaluationCode::ResidualValue,
                "a symbolic compile-time expression cannot be used as a frozen value",
            )),
            // compile-time-runtime-dependency
            EvalOutcome::RuntimeDependency(_) => Err(Diagnostic::new(
                EvaluationCode::RuntimeDependency,
                "expression depends on a runtime value",
            )),
            // compile-time-cancelled
            EvalOutcome::Cancelled => Err(Diagnostic::new(
                EvaluationCode::Cancelled,
                "compile-time evaluation was cancelled",
            )),
            EvalOutcome::Trap(diagnostic) => Err(diagnostic),
        }
    }

    fn reference_aggregate_value(
        &mut self,
        expr: ExprId,
        args: &[ExprId],
    ) -> EvalResult<Option<Address>> {
        let has_references = self
            .program
            .type_registry
            .contains_references(&self.program.exprs.ty[expr.index()]);
        if !has_references {
            return Ok(None);
        }
        let mut fields = Vec::new();
        for argument in args {
            fields.push(match self.value(*argument)? {
                EvalValue::Frozen(value) => self.heap.allocate(value),
                EvalValue::Reference(address) => self.heap.allocate_reference(address),
            });
        }
        Ok(Some(self.heap.allocate_aggregate(fields)))
    }

    fn index(&mut self, expr: ExprId) -> EvalResult<usize> {
        self.complete(expr)?
            .as_const_size_int()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| {
                // compile-time-invalid-index
                Diagnostic::new(
                    EvaluationCode::InvalidIndex,
                    "index must be a nonnegative integer",
                )
            })
    }

    fn index_address(&mut self, buf: ExprId, base: Address, index: usize) -> EvalResult<Address> {
        let ty = &self.program.exprs.ty[buf.as_usize()];
        let ty = crate::ty::reference_pointee_for_type(ty, Some(self.program.type_registry))
            .unwrap_or_else(|| ty.clone());
        if matches!(
            self.program.type_registry.resolved_definition_for_type(&ty),
            Ty::Array { .. }
        ) || crate::representation::Repr::array_parts(&ty, Some(self.program.type_registry))
            .is_some()
        {
            self.heap.project(base, index)
        } else {
            let offset = isize::try_from(index).map_err(|_| {
                // compile-time-address-overflow
                Diagnostic::new(
                    EvaluationCode::AddressOverflow,
                    "index exceeds virtual address space",
                )
            })?;
            base.offset(offset)
        }
    }

    fn place(&mut self, expr: ExprId) -> EvalResult<Address> {
        match self.program.exprs.kind_of(expr) {
            Some(TypedExprKind::FnCall {
                target, args: None, ..
            }) if self.local(target.0).is_some() => match self.local(target.0).unwrap() {
                LocalValue::Stored(address) | LocalValue::Reference(address) => Ok(address),
            },
            Some(TypedExprKind::Bind { name, .. }) => {
                let name = *name;
                if self.local(name).is_none() {
                    self.evaluated_value(expr)?;
                }
                match self.local(name) {
                    Some(LocalValue::Stored(address) | LocalValue::Reference(address)) => {
                        Ok(address)
                    }
                    // compile-time-invalid-address
                    None => Err(Diagnostic::new(
                        EvaluationCode::InvalidAddress,
                        "binding has no active storage",
                    )),
                }
            }
            Some(TypedExprKind::Ref(inner) | TypedExprKind::TakePtr(inner)) => self.place(*inner),
            Some(TypedExprKind::AggregateGet { base, index }) => {
                let base = self.place(*base)?;
                self.heap.project(base, *index)
            }
            Some(TypedExprKind::SliceGet { buf, start, length }) => {
                let base = match self.value(*buf)? {
                    EvalValue::Reference(address) => address,
                    EvalValue::Frozen(value) => self.heap.allocate(value),
                };
                let start = self.index(*start)?;
                let length = self.index(*length)?;
                self.heap.slice(base, start, length)
            }
            Some(TypedExprKind::BufGet { buf, index }) => {
                let base = match self.value(*buf)? {
                    EvalValue::Reference(address) => address,
                    EvalValue::Frozen(value) => self.heap.allocate(value),
                };
                let index = self.index(*index)?;
                self.index_address(*buf, base, index)
            }
            Some(TypedExprKind::Deref(inner)) => match self.value(*inner)? {
                EvalValue::Reference(address) => Ok(address),
                // compile-time-invalid-deref
                _ => Err(Diagnostic::new(
                    EvaluationCode::InvalidDeref,
                    "cannot dereference a non-address compile-time value",
                )),
            },
            _ => match self.value(expr)? {
                EvalValue::Reference(address) => Ok(address),
                EvalValue::Frozen(value) => Ok(self.heap.allocate(value)),
            },
        }
    }

    fn replace_at(
        &mut self,
        address: Address,
        value: ExprId,
        operator: Option<&crate::ResolvedCompoundOperator>,
    ) -> EvalResult<ConstValue> {
        let replacement = self.complete(value)?;
        if self.returned.is_some() {
            return Ok(replacement);
        }
        let replacement = if let Some(operator) = operator {
            let current = self.heap.freeze(address)?;
            match operator {
                crate::ResolvedCompoundOperator::Callable { target, .. } => {
                    self.call(*target, &[current, replacement])?
                }
                _ => {
                    // compile-time-invalid-reassignment
                    return Err(Diagnostic::new(
                        EvaluationCode::InvalidReassignment,
                        "compound operator cannot be evaluated",
                    ));
                }
            }
        } else {
            replacement
        };
        self.heap.store(address, replacement.clone())?;
        Ok(replacement)
    }

    fn match_and_bind(&mut self, pattern: &ast::Pattern, value: &ConstValue) -> EvalResult<bool> {
        if !self.pattern_matches(pattern, value)? {
            return Ok(false);
        }
        let mut bindings = HashMap::new();
        crate::analysis::pattern::collect_pattern_bindings(pattern, value, &mut bindings);
        for (name, value) in bindings {
            let local = self.store_local(EvalValue::Frozen(value));
            if let Some(frame) = self.locals.last_mut() {
                frame.insert(name, local);
            }
        }
        Ok(true)
    }

    fn pattern_matches(&self, pattern: &ast::Pattern, value: &ConstValue) -> EvalResult<bool> {
        let validity = match pattern {
            ast::Pattern::InRange {
                bounds: ast::InRangeBounds::Tag(name),
                ..
            } => self
                .program
                .tags
                .get(&crate::typed::TagId(*name))
                .and_then(|tag| {
                    self.program
                        .type_registry
                        .integer_validity_for_type(&tag.resolved_ty)
                }),
            _ => None,
        };
        crate::analysis::pattern::pattern_matches_with_integer_validity(pattern, value, validity)
            .ok_or_else(|| {
                // compile-time-integer-domain-unresolved
                Diagnostic::new(
                    EvaluationCode::IntegerDomainUnresolved,
                    "integer domain membership requires a resolved complete predicate",
                )
            })
    }

    fn condition_matches(&mut self, condition: &crate::typed::TypedCondition) -> EvalResult<bool> {
        Ok(match condition {
            crate::typed::TypedCondition::Proposition { subject } => {
                let ConstValue::Proposition(holds) = self.complete(*subject)? else {
                    // type-condition-requires-proposition
                    return Err(Diagnostic::new(
                        PropositionCode::ConditionRequired,
                        "conditions require a proposition",
                    )
                    .into());
                };
                holds
            }
            crate::typed::TypedCondition::Compare {
                left,
                relation,
                right,
            } => {
                let ConstValue::Int(left) = self.complete(*left)? else {
                    // type-numeric-predicate-operands
                    return Err(Diagnostic::new(
                        PropositionCode::NumericOperandsRequired,
                        "numeric predicates require integer operands",
                    )
                    .into());
                };
                let ConstValue::Int(right) = self.complete(*right)? else {
                    // type-numeric-predicate-operands
                    return Err(Diagnostic::new(
                        PropositionCode::NumericOperandsRequired,
                        "numeric predicates require integer operands",
                    )
                    .into());
                };
                relation.holds(left, right)
            }
            crate::typed::TypedCondition::Is { subject, pattern } => {
                let value = self.complete(*subject)?;
                self.match_and_bind(&pattern.value, &value)?
            }
            crate::typed::TypedCondition::Not(inner) => !self.condition_matches(inner)?,
            crate::typed::TypedCondition::And(left, right) => {
                self.condition_matches(left)? && self.condition_matches(right)?
            }
            crate::typed::TypedCondition::Or(left, right) => {
                self.condition_matches(left)? || self.condition_matches(right)?
            }
        })
    }

    pub fn call(
        &mut self,
        target: crate::typed::DefId,
        args: &[ConstValue],
    ) -> EvalResult<ConstValue> {
        if self.frames.is_empty() {
            self.failure_frames = None;
        }
        let result = self
            .call_values(
                target,
                &args
                    .iter()
                    .cloned()
                    .map(EvalValue::Frozen)
                    .collect::<Vec<_>>(),
            )
            .and_then(EvalValue::freeze);
        if result.is_ok() {
            self.failure_frames = None;
        }
        result
    }

    pub fn call_with_state(
        &mut self,
        target: crate::typed::DefId,
        args: &[ConstValue],
        state: &mut EvalState,
    ) -> EvalResult<ConstValue> {
        std::mem::swap(&mut self.heap, &mut state.heap);
        std::mem::swap(&mut self.fuel, &mut state.fuel);
        let result = self.call(target, args);
        std::mem::swap(&mut self.heap, &mut state.heap);
        std::mem::swap(&mut self.fuel, &mut state.fuel);
        result
    }

    fn eval_local_call(&mut self, call: ExprId, definition: ExprId) -> EvalResult<EvalOutcome> {
        let TypedExprKind::FnCall { args, .. } = &self.program.exprs.kind[call.index()] else {
            unreachable!()
        };
        let arguments = args
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|id| self.value(*id))
            .collect::<EvalResult<Vec<_>>>()?;
        if let Some(value) = &self.returned {
            return Ok(value.clone().outcome());
        }
        let parameters = self
            .program
            .local_parameters(definition)
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        let TypedExprKind::Bind { stmts, body, .. } = &self.program.exprs.kind[definition.index()]
        else {
            unreachable!()
        };
        let mut frame = HashMap::new();
        for (name, value) in parameters.into_iter().zip(arguments) {
            frame.insert(name, self.store_local(value));
        }
        self.locals.push(frame);
        let caller_return = self.returned.take();
        let caller_subjects = std::mem::take(&mut self.when_subjects);
        let result = self.eval_returning_sequence(stmts, Some(*body));
        self.returned = caller_return;
        self.when_subjects = caller_subjects;
        self.locals.pop();
        result.map(EvalValue::outcome)
    }

    fn eval_returning_sequence(
        &mut self,
        exprs: &[ExprId],
        ret: Option<ExprId>,
    ) -> EvalResult<EvalValue> {
        for expr in exprs {
            self.evaluated_value(*expr)?;
            if let Some(value) = &self.returned {
                return Ok(value.clone());
            }
        }
        ret.map(|expr| self.value(expr)).unwrap_or({
            Ok(EvalValue::Frozen(ConstValue::Record {
                fields: Vec::new().into(),
            }))
        })
    }

    fn call_values(
        &mut self,
        target: crate::typed::DefId,
        args: &[EvalValue],
    ) -> EvalResult<EvalValue> {
        if !self.program.defs.contains_key(&target)
            && let Some(program) = self
                .programs
                .iter()
                .find(|program| program.defs.contains_key(&target))
                .copied()
        {
            let caller = std::mem::replace(&mut self.program, program);
            let result = self.call_values(target, args);
            self.program = caller;
            return result;
        }
        let bind = self.program.defs.get(&target);
        let Some(bind) = bind else {
            // compile-time-unknown-function
            return Err(Diagnostic::new(
                EvaluationCode::UnknownFunction,
                format!("callable `{}` has no definition", target.0),
            ));
        };
        if bind.params.len() != args.len() {
            // compile-time-arity-mismatch
            return Err(Diagnostic::new(
                EvaluationCode::ArityMismatch,
                "compile-time function call has the wrong number of arguments",
            ));
        }
        let mut frame = HashMap::new();
        for ((name, _), value) in bind.params.iter().zip(args) {
            let value = self.store_local(value.clone());
            frame.insert(*name, value);
            if name.as_str() == "self" {
                frame.insert(target.0, value);
            }
        }
        self.call_body(target, bind, frame)
    }

    fn call_body(
        &mut self,
        target: crate::DefId,
        bind: &crate::TypedBind,
        frame: HashMap<Intern<String>, LocalValue>,
    ) -> EvalResult<EvalValue> {
        if self.frames.is_empty() {
            self.failure_frames = None;
        }
        self.frames.push(ActiveFrame {
            function: Some(target),
            file: self.program.file_id,
            expression: None,
            caller_locals: std::mem::replace(&mut self.locals, vec![frame]),
        });
        let caller_return = self.returned.take();
        let caller_subjects = std::mem::take(&mut self.when_subjects);
        let result = match &bind.body {
            crate::typed::BindBody::Expr(expr) => self.value(*expr),
            crate::typed::BindBody::Body { exprs, ret } => {
                self.eval_returning_sequence(exprs, *ret)
            }
            // compile-time-missing-body
            crate::typed::BindBody::Unlowered => Err(Diagnostic::new(
                EvaluationCode::MissingBody,
                "callable has no lowered body",
            )),
        };
        if result.is_err()
            || (self.frames.len() == 1 && matches!(result, Ok(EvalValue::Reference(_))))
        {
            self.capture_frames();
        }
        self.returned = caller_return;
        self.when_subjects = caller_subjects;
        self.locals = self.frames.pop().expect("active call frame").caller_locals;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_receivers_use_the_active_call_frame() {
        use crate::transform::{TransformCtx, transform};
        use parser::cursor::TokenCursor;
        let source = format!(
            "{}    copy(self ref) Self: return deref self\ncopy_value(value Byte) Byte: return value.copy\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let typed = transform(
            &TokenCursor::parse_source(&source),
            crate::FileId(0),
            &TransformCtx::new(),
        );
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let receiver = typed
            .exprs
            .kind
            .iter()
            .position(|kind| matches!(kind, TypedExprKind::SelfRef { .. }))
            .unwrap();
        let receiver = ExprId(u32::try_from(receiver).unwrap());
        let target = flask::CompileTarget::Library;
        let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        assert!(
            matches!(evaluator.eval(receiver), Ok(EvalOutcome::RuntimeDependency(expr)) if expr == receiver)
        );
        for value in [0, 17, 255] {
            assert_eq!(
                evaluator.call(
                    crate::DefId::named(Intern::from_ref("copy_value")),
                    &[ConstValue::Int(value.into())]
                ),
                Ok(ConstValue::Int(value.into()))
            );
        }
    }

    #[test]
    fn conditional_returns_stop_evaluation_only_on_the_taken_branch() {
        use crate::transform::{TransformCtx, transform};
        use parser::cursor::TokenCursor;

        let source = format!(
            "{}choose(outer Word, inner Word) Word:\n    when outer is 1 then\n        value Word: 17\n        when inner is 1 then\n            return 11\n        return value\n    return 29\nlooping(flag Word) Word:\n    while flag is 1 do\n        when flag is 1 then\n            return 17\n    return 29\n",
            test_fixtures::preludes::WORD_PRELUDE
        );
        let typed = transform(
            &TokenCursor::parse_source(&source),
            crate::FileId(0),
            &TransformCtx::new(),
        );
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let target = flask::CompileTarget::Library;
        let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        for (outer, inner, expected) in [(0, 0, 29), (1, 0, 17), (1, 1, 11), (0, 1, 29)] {
            assert_eq!(
                evaluator.call(
                    crate::DefId::named(Intern::from_ref("choose")),
                    &[ConstValue::Int(outer.into()), ConstValue::Int(inner.into())]
                ),
                Ok(ConstValue::Int(expected.into())),
            );
        }
        for (flag, expected) in [(1, 17), (0, 29)] {
            assert_eq!(
                evaluator.call(
                    crate::DefId::named(Intern::from_ref("looping")),
                    &[ConstValue::Int(flag.into())]
                ),
                Ok(ConstValue::Int(expected.into())),
            );
        }
    }

    #[test]
    fn calls_across_files_restore_the_callers_expression_arena() {
        use crate::transform::{PackageTransformOptions, TransformCtx, transform_package};
        use parser::cursor::TokenCursor;

        let files = [
            format!(
                "{}echo(value Word) Word: return value\n",
                test_fixtures::preludes::WORD_PRELUDE
            ),
            "forward(value Word) Word: return echo(value)\n".to_string(),
            "outer(value Word) Word: return forward(value)\nlocal(value Word) Word: return value\n"
                .to_string(),
        ];
        let parsed: Vec<_> = files
            .iter()
            .enumerate()
            .map(|(index, source)| {
                (
                    TokenCursor::parse_source(source),
                    crate::FileId(index as u32),
                )
            })
            .collect();
        let typed = transform_package(&parsed, &TransformCtx::new(), PackageTransformOptions::IDE);
        for file in &typed {
            assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
        }
        let target = flask::CompileTarget::Library;
        let mut evaluator =
            Evaluator::new(typed[2].resolved_program(), CompileTimeHost::new(&target))
                .with_programs(typed.iter().map(crate::TypedFileAst::resolved_program));
        for name in ["outer", "local", "outer"] {
            assert_eq!(
                evaluator.call(
                    crate::DefId::named(Intern::from_ref(name)),
                    &[ConstValue::ONE]
                ),
                Ok(ConstValue::ONE)
            );
        }
        assert_eq!(evaluator.program.file_id, typed[2].file_id);
        assert!(evaluator.locals.is_empty());
        let error = evaluator
            .call(crate::DefId::named(Intern::from_ref("echo")), &[])
            .unwrap_err();
        assert_eq!(error.code.slug().as_str(), "compile-time-arity-mismatch");
        assert_eq!(evaluator.program.file_id, typed[2].file_id);
        assert!(evaluator.locals.is_empty());
    }

    #[test]
    fn virtual_heap_round_trips_frozen_values() {
        let mut heap = EvalHeap::default();
        let address = heap.allocate(ConstValue::ZERO);
        assert_eq!(heap.load(address), Some(&ConstValue::ZERO));
        heap.store(address, ConstValue::ONE).unwrap();
        assert_eq!(heap.freeze(address), Ok(ConstValue::ONE));
    }

    #[test]
    fn virtual_heap_rejects_invalid_addresses() {
        let mut heap = EvalHeap::default();
        let invalid = heap.allocate(ConstValue::ZERO).offset(1).unwrap();
        assert!(heap.load(invalid).is_none());
        assert!(heap.store(invalid, ConstValue::ZERO).is_err());
        assert!(heap.freeze(invalid).is_err());
    }

    #[test]
    fn virtual_heap_supports_contiguous_offsets() {
        let mut heap = EvalHeap::default();
        let base = heap.allocate(ConstValue::FixedArray(
            [ConstValue::ZERO, ConstValue::ONE].into(),
        ));
        let element = heap.project(base, 1).unwrap();
        assert_eq!(heap.load(element), Some(&ConstValue::ONE));
        heap.store(element, ConstValue::ZERO).unwrap();
        assert_eq!(heap.load(element), Some(&ConstValue::ZERO));
        assert!(heap.project(base, 2).is_err());
    }
}

#[cfg(test)]
#[path = "eval/scope_tests.rs"]
mod scope_tests;

#[cfg(test)]
#[path = "eval/frame_tests.rs"]
mod frame_tests;

#[cfg(test)]
#[path = "eval/address_tests.rs"]
mod address_tests;
