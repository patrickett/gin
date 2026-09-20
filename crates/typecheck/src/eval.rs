//! Execution boundary for resolved expressions.
//!
//! This is the common entry point for compile-time evaluation and the
//! interactive interpreter. It executes resolved typed expressions and keeps
//! effects behind an explicit host boundary.

use ast::{ConstValue, Literal};
use diagnostic::Diagnostic;
use internment::Intern;
use std::collections::HashMap;

use crate::resolved_program::ResolvedProgram;
use crate::ty::Ty;
use crate::typed::{ExprId, TypedExprKind};

mod heap;
pub use heap::{Address, EvalHeap};

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
            Self::Reference(_) => Err(Diagnostic::new(
                "compile-time-address-value",
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

    pub fn with_heap(mut self, heap: EvalHeap) -> Self {
        self.heap = heap;
        self
    }

    pub fn into_heap(self) -> EvalHeap {
        self.heap
    }

    pub fn eval_with_state(
        &mut self,
        expr: ExprId,
        state: &mut EvalState,
    ) -> EvalResult<EvalOutcome> {
        std::mem::swap(&mut self.heap, &mut state.heap);
        std::mem::swap(&mut self.fuel, &mut state.fuel);
        let result = self.eval(expr);
        std::mem::swap(&mut self.heap, &mut state.heap);
        std::mem::swap(&mut self.fuel, &mut state.fuel);
        result
    }

    pub fn heap(&self) -> &EvalHeap {
        &self.heap
    }

    pub fn heap_mut(&mut self) -> &mut EvalHeap {
        &mut self.heap
    }

    pub fn eval(&mut self, expr: ExprId) -> EvalResult<EvalOutcome> {
        if self.host.cancelled() {
            return Ok(EvalOutcome::Cancelled);
        }
        if !self.fuel.consume() {
            return Err(Diagnostic::new(
                "compile-time-fuel-exhausted",
                "compile-time evaluation exceeded its fuel limit",
            ));
        }

        let Some(kind) = self.program.exprs.kind_of(expr) else {
            return Err(Diagnostic::new(
                "compile-time-invalid-expression",
                "expression is not present in the resolved program",
            ));
        };

        let direct = match kind {
            TypedExprKind::Register(_) => None,
            TypedExprKind::Invalid => None,
            TypedExprKind::Lit(Literal::Int(value)) => Some(ConstValue::Int(*value)),
            TypedExprKind::Lit(Literal::Float(value)) => Some(ConstValue::Float(value.clone())),
            TypedExprKind::Lit(Literal::String(value)) => Some(ConstValue::Text(value.clone())),
            TypedExprKind::Lit(Literal::Number(value)) => {
                Some(ConstValue::Int((*value as u128).into()))
            }
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
                return Ok(EvalOutcome::Trap(Diagnostic::new(
                    "compile-time-invalid-operator",
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
                        return Err(Diagnostic::new(
                            "compile-time-invalid-intrinsic",
                            "address intrinsic is missing its address operand",
                        ));
                    };
                    let address = match self.eval(*address_expr)? {
                        EvalOutcome::Address(address) => address,
                        EvalOutcome::RuntimeDependency(expr) => {
                            return Ok(EvalOutcome::RuntimeDependency(expr));
                        }
                        EvalOutcome::Residual(expr) => return Ok(EvalOutcome::Residual(expr)),
                        EvalOutcome::Cancelled => return Ok(EvalOutcome::Cancelled),
                        EvalOutcome::Trap(diagnostic) => return Ok(EvalOutcome::Trap(diagnostic)),
                        EvalOutcome::Complete(_) => {
                            return Err(Diagnostic::new(
                                "compile-time-invalid-address",
                                "address intrinsic requires a virtual address",
                            ));
                        }
                    };
                    return Ok(EvalOutcome::Complete(match op {
                        crate::intrinsic::IntrinsicOp::AddressLoad => self.heap.freeze(address)?,
                        crate::intrinsic::IntrinsicOp::AddressStore => {
                            let Some(value_expr) = args.get(1) else {
                                return Err(Diagnostic::new(
                                    "compile-time-invalid-intrinsic",
                                    "address store is missing its value operand",
                                ));
                            };
                            let value = self.complete(*value_expr)?;
                            self.heap.store(address, value.clone())?;
                            value
                        }
                        crate::intrinsic::IntrinsicOp::AddressOffset => {
                            let Some(offset_expr) = args.get(1) else {
                                return Err(Diagnostic::new(
                                    "compile-time-invalid-intrinsic",
                                    "address offset is missing its offset operand",
                                ));
                            };
                            let offset = self
                                .complete(*offset_expr)?
                                .as_const_size_int()
                                .ok_or_else(|| {
                                    Diagnostic::new(
                                        "compile-time-invalid-address-offset",
                                        "virtual address offset is not an integer",
                                    )
                                })?;
                            let offset = isize::try_from(offset).map_err(|_| {
                                Diagnostic::new(
                                    "compile-time-invalid-address-offset",
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
                let source_width = args.first().and_then(|arg| {
                    self.program
                        .type_registry
                        .integer_width_for_type(&self.program.exprs.ty[arg.as_usize()])
                });
                let result_width = self
                    .program
                    .type_registry
                    .integer_width_for_type(&self.program.exprs.ty[expr.as_usize()]);
                let (Some(source_width), Some(result_width)) = (source_width, result_width) else {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                };
                Some(
                    op.fold(&values, source_width, result_width)
                        .map_err(|error| op.diagnostic(&error))?
                        .ok_or_else(|| {
                            Diagnostic::new(
                                "compile-time-unsupported-intrinsic",
                                "intrinsic is not supported by the compile-time evaluator",
                            )
                        })?,
                )
            }
            TypedExprKind::UnionPayload { expr: inner, .. } => {
                let ConstValue::Tag { args, .. } = self.complete(*inner)? else {
                    return Err(Diagnostic::new(
                        "compile-time-invalid-union-payload",
                        "expected a tagged union value",
                    ));
                };
                args.first().cloned()
            }
            TypedExprKind::Cast { expr: inner, ty } => {
                let value = self.complete(*inner)?;
                let source_ty = &self.program.exprs.ty[inner.as_usize()];
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
                                Diagnostic::new(
                                    "compile-time-unsupported-cast",
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
                let value = (|| {
                    for stmt in stmts {
                        let _ = self.eval(*stmt)?;
                    }
                    self.value(*body)
                })();
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
                    Diagnostic::new(
                        "compile-time-unassigned-value",
                        "compile-time reassignment refers to an unassigned value",
                    )
                })?;
                let replacement = self.value(*value)?;
                let replacement = if let Some(operator) = operator {
                    let current = self.read_local(local)?.freeze()?;
                    EvalValue::Frozen(
                        current
                            .eval_binop(operator, &replacement.freeze()?)
                            .ok_or_else(|| {
                                Diagnostic::new(
                                    "compile-time-invalid-reassignment",
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
                        return Err(Diagnostic::new(
                            "compile-time-invalid-destructure",
                            "compile-time destructuring requires a matching record or tag value",
                        ));
                    }
                };
                let Some(fields) = fields else {
                    return Err(Diagnostic::new(
                        "compile-time-invalid-destructure",
                        "compile-time destructuring type is not a record or union",
                    ));
                };
                let Some(locals) = self.locals.last_mut() else {
                    return Err(Diagnostic::new(
                        "compile-time-invalid-destructure",
                        "compile-time destructuring has no active local scope",
                    ));
                };
                for (field_name, binding_name) in field_bindings {
                    let Some(index) = fields.iter().position(|name| name == field_name) else {
                        return Err(Diagnostic::new(
                            "compile-time-unknown-field",
                            "compile-time destructuring field does not exist",
                        ));
                    };
                    let Some(value) = values.get(index) else {
                        return Err(Diagnostic::new(
                            "compile-time-invalid-destructure",
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
            } => Some(ConstValue::Tag {
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
            }),
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
                    Diagnostic::new(
                        "compile-time-invalid-allocation",
                        "fixed-array allocation size is invalid",
                    )
                })?;
                Some(ConstValue::FixedArray(vec![value; count].into()))
            }
            TypedExprKind::Deref(inner) => {
                let address = match self.eval(*inner)? {
                    EvalOutcome::Address(address) => address,
                    EvalOutcome::RuntimeDependency(expr) => {
                        return Ok(EvalOutcome::RuntimeDependency(expr));
                    }
                    EvalOutcome::Residual(expr) => return Ok(EvalOutcome::Residual(expr)),
                    EvalOutcome::Cancelled => return Ok(EvalOutcome::Cancelled),
                    EvalOutcome::Trap(diagnostic) => return Ok(EvalOutcome::Trap(diagnostic)),
                    EvalOutcome::Complete(_) => {
                        return Err(Diagnostic::new(
                            "compile-time-invalid-deref",
                            "cannot dereference a non-address compile-time value",
                        ));
                    }
                };
                Some(self.heap.freeze(address)?)
            }
            TypedExprKind::AggregateGet { base, index } => match self.eval(*base)? {
                EvalOutcome::Address(address) => {
                    let field = self.heap.project(address, *index)?;
                    Some(self.heap.freeze(field)?)
                }
                EvalOutcome::Complete(ConstValue::ResultRow(items)) => items.get(*index).cloned(),
                EvalOutcome::Complete(ConstValue::Tag { args, .. }) => args.get(*index).cloned(),
                EvalOutcome::Complete(ConstValue::Record { fields }) => {
                    fields.get(*index).map(|(_, value)| value.clone())
                }
                EvalOutcome::Complete(_) => None,
                EvalOutcome::RuntimeDependency(expr) => {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                }
                EvalOutcome::Residual(expr) => return Ok(EvalOutcome::Residual(expr)),
                EvalOutcome::Cancelled => return Ok(EvalOutcome::Cancelled),
                EvalOutcome::Trap(diagnostic) => return Ok(EvalOutcome::Trap(diagnostic)),
            },
            TypedExprKind::BufGet { buf, index } => {
                let index = self.complete(*index)?.as_const_size_int().ok_or_else(|| {
                    Diagnostic::new(
                        "compile-time-invalid-index",
                        "buffer index is not an integer",
                    )
                })?;
                let index = usize::try_from(index).map_err(|_| {
                    Diagnostic::new("compile-time-invalid-index", "buffer index is negative")
                })?;
                match self.eval(*buf)? {
                    EvalOutcome::Address(address) => {
                        let element = self.index_address(*buf, address, index)?;
                        Some(self.heap.freeze(element)?)
                    }
                    EvalOutcome::Complete(
                        ConstValue::FixedArray(items) | ConstValue::Sequence(items),
                    ) => Some(items.get(index).cloned().ok_or_else(|| {
                        Diagnostic::new(
                            "compile-time-index-out-of-bounds",
                            "buffer index is outside the initialized aggregate",
                        )
                    })?),
                    EvalOutcome::Complete(ConstValue::Text(text)) => text
                        .as_bytes()
                        .get(index)
                        .map(|byte| ConstValue::Int((*byte as u128).into())),
                    EvalOutcome::Complete(_) => None,
                    EvalOutcome::RuntimeDependency(expr) => {
                        return Ok(EvalOutcome::RuntimeDependency(expr));
                    }
                    EvalOutcome::Residual(expr) => return Ok(EvalOutcome::Residual(expr)),
                    EvalOutcome::Cancelled => return Ok(EvalOutcome::Cancelled),
                    EvalOutcome::Trap(diagnostic) => return Ok(EvalOutcome::Trap(diagnostic)),
                }
            }
            TypedExprKind::If(if_expr) => {
                if self.condition_matches(&if_expr.condition)? {
                    for stmt in &if_expr.stmts {
                        let _ = self.eval(*stmt)?;
                    }
                    if let Some(ret) = if_expr.ret {
                        Some(self.complete(ret)?)
                    } else {
                        Some(ConstValue::Record {
                            fields: Vec::new().into(),
                        })
                    }
                } else {
                    None
                }
            }
            TypedExprKind::When(when_expr) => {
                let subject = when_expr
                    .subject
                    .map(|subject| self.complete(subject))
                    .transpose()?
                    .unwrap_or(ConstValue::ZERO);
                let mut selected = None;
                for arm in &when_expr.arms {
                    let body = match arm {
                        crate::typed::TypedWhenArm::Cond {
                            condition, body, ..
                        } => self.condition_matches(condition)?.then_some(*body),
                        crate::typed::TypedWhenArm::Is { pattern, body, .. } => {
                            crate::analysis::pattern::pattern_matches_public(
                                &pattern.value,
                                &subject,
                            )
                            .then_some(*body)
                        }
                        crate::typed::TypedWhenArm::Else(body, _) => Some(*body),
                    };
                    if body.is_some() {
                        selected = body;
                        break;
                    }
                }
                Some(self.complete(selected.ok_or_else(|| {
                    Diagnostic::new(
                        "compile-time-no-when-arm",
                        "no compile-time when arm matched",
                    )
                })?)?)
            }
            TypedExprKind::Loop(loop_expr) => {
                match &loop_expr.kind {
                    crate::typed::TypedLoopKind::While { condition } => {
                        while self.condition_matches(condition)? {
                            for stmt in &loop_expr.stmts {
                                let _ = self.eval(*stmt)?;
                            }
                        }
                    }
                    crate::typed::TypedLoopKind::ForIn {
                        variable, iterable, ..
                    } => {
                        let values = match self.complete(*iterable)? {
                            ConstValue::Sequence(values) => values.to_vec(),
                            value => vec![value],
                        };
                        for value in values {
                            {
                                let Some(locals) = self.locals.last_mut() else {
                                    return Err(Diagnostic::new(
                                        "compile-time-invalid-loop",
                                        "compile-time for-loop has no active local scope",
                                    ));
                                };
                                locals.insert(
                                    *variable,
                                    LocalValue::Stored(self.heap.allocate(value)),
                                );
                            }
                            for stmt in &loop_expr.stmts {
                                let _ = self.eval(*stmt)?;
                            }
                        }
                    }
                }
                Some(ConstValue::Record {
                    fields: Vec::new().into(),
                })
            }
            TypedExprKind::Range { start, end } => {
                let start = self.complete(*start)?.as_const_size_int();
                let end = self.complete(*end)?.as_const_size_int();
                let (Some(start), Some(end)) = (start, end) else {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                };
                let mut values = Vec::new();
                let mut current = start;
                while current < end {
                    if !self.fuel.consume() {
                        return Err(Diagnostic::new(
                            "compile-time-fuel-exhausted",
                            "compile-time range evaluation exceeded its fuel limit",
                        ));
                    }
                    values.push(ConstValue::Int(i256::I256::from(current)));
                    current = current.checked_add(1).ok_or_else(|| {
                        Diagnostic::new(
                            "compile-time-range-overflow",
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
                base, field, value, ..
            } => {
                let address = self.place(*base)?;
                let replacement = self.complete(*value)?;
                let current = self.heap.freeze(address)?;
                let ConstValue::Record { fields } = current else {
                    return Err(Diagnostic::new(
                        "compile-time-invalid-store",
                        "record store requires a heap-resident record",
                    ));
                };
                let mut fields = fields.to_vec();
                let Some((_, slot)) = fields.iter_mut().find(|(name, _)| name == field) else {
                    return Err(Diagnostic::new(
                        "compile-time-unknown-field",
                        "record store field does not exist",
                    ));
                };
                *slot = replacement.clone();
                self.heap.store(
                    address,
                    ConstValue::Record {
                        fields: fields.into(),
                    },
                )?;
                Some(replacement)
            }
            TypedExprKind::Target => match self.host.target() {
                flask::CompileTarget::Concrete(target) => Some(ConstValue::Record {
                    fields: vec![
                        (
                            internment::Intern::from_ref("arch"),
                            ConstValue::Text(target.arch.as_str().into()),
                        ),
                        (
                            internment::Intern::from_ref("vendor"),
                            ConstValue::Text(target.vendor.clone()),
                        ),
                        (
                            internment::Intern::from_ref("os"),
                            ConstValue::Text(target.os.as_str().into()),
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
                    "arch" => Some(ConstValue::Text(target.arch.as_str().into())),
                    "vendor" => Some(ConstValue::Text(target.vendor.clone())),
                    "os" => Some(ConstValue::Text(target.os.as_str().into())),
                    "triple" => Some(ConstValue::Text(target.raw.clone())),
                    _ => None,
                },
                flask::CompileTarget::Library => None,
            },
            TypedExprKind::TargetQuery { kind, operand } => {
                let layout = self.program.target_layout.as_ref().ok_or_else(|| {
                    Diagnostic::new(
                        "compile-time-target-unavailable",
                        "target queries require a concrete compile target",
                    )
                })?;
                Some(ConstValue::Int(i256::I256::from(
                    layout.query(*kind, operand, None).map_err(|_| {
                        Diagnostic::new(
                            "compile-time-target-query-failed",
                            "target layout could not answer the compile-time query",
                        )
                    })?,
                )))
            }
            TypedExprKind::Asm {
                state: crate::assembly::AssemblyState::Checked(assembly),
                ..
            } if assembly.evaluation.is_some() => {
                let method = assembly.evaluation.unwrap();
                if self.program.defs.get(&method).is_none_or(|bind| {
                    bind.call_capability == crate::typed::CallCapability::RuntimeOnly
                }) {
                    return Ok(EvalOutcome::RuntimeDependency(expr));
                }
                let values = assembly
                    .operands
                    .iter()
                    .map(|(_, operand)| self.complete(*operand))
                    .collect::<EvalResult<Vec<_>>>()?;
                Some(self.call(method, &values)?)
            }
            TypedExprKind::Asm { .. } | TypedExprKind::SelfRef { .. } => {
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
            Some(None) if matches!(kind, TypedExprKind::Bind { .. }) => Err(Diagnostic::new(
                "compile-time-unresolved-expression",
                "typed expression could not be evaluated",
            )),
            Some(None) => Ok(EvalOutcome::RuntimeDependency(expr)),
            None => unreachable!("expression was checked above"),
        }
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

    fn complete(&mut self, expr: ExprId) -> EvalResult<ConstValue> {
        self.value(expr)?.freeze()
    }

    fn value(&mut self, expr: ExprId) -> EvalResult<EvalValue> {
        if let Some(TypedExprKind::Bind { name, .. }) = self.program.exprs.kind_of(expr)
            && let Some(local) = self.local(*name)
        {
            return self.read_local(local);
        }
        match self.eval(expr)? {
            EvalOutcome::Complete(value) => Ok(EvalValue::Frozen(value)),
            EvalOutcome::Address(address) => Ok(EvalValue::Reference(address)),
            EvalOutcome::Residual(_) => Err(Diagnostic::new(
                "compile-time-residual-value",
                "a symbolic compile-time expression cannot be used as a frozen value",
            )),
            EvalOutcome::RuntimeDependency(_) => Err(Diagnostic::new(
                "compile-time-runtime-dependency",
                "expression depends on a runtime value",
            )),
            EvalOutcome::Cancelled => Err(Diagnostic::new(
                "compile-time-cancelled",
                "compile-time evaluation was cancelled",
            )),
            EvalOutcome::Trap(diagnostic) => Err(diagnostic),
        }
    }

    fn index(&mut self, expr: ExprId) -> EvalResult<usize> {
        self.complete(expr)?
            .as_const_size_int()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| {
                Diagnostic::new(
                    "compile-time-invalid-index",
                    "index must be a nonnegative integer",
                )
            })
    }

    fn index_address(&mut self, buf: ExprId, base: Address, index: usize) -> EvalResult<Address> {
        let ty = &self.program.exprs.ty[buf.as_usize()];
        let ty = crate::ty::reference_pointee_for_type(ty, Some(self.program.type_registry))
            .unwrap_or_else(|| ty.clone());
        if crate::representation::Repr::array_parts(&ty, Some(self.program.type_registry)).is_some()
        {
            self.heap.project(base, index)
        } else {
            let offset = isize::try_from(index).map_err(|_| {
                Diagnostic::new(
                    "compile-time-address-overflow",
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
                    self.eval(expr)?;
                }
                match self.local(name) {
                    Some(LocalValue::Stored(address) | LocalValue::Reference(address)) => {
                        Ok(address)
                    }
                    None => Err(Diagnostic::new(
                        "compile-time-invalid-address",
                        "binding has no active storage",
                    )),
                }
            }
            Some(TypedExprKind::Ref(inner) | TypedExprKind::TakePtr(inner)) => self.place(*inner),
            Some(TypedExprKind::AggregateGet { base, index }) => {
                let base = self.place(*base)?;
                self.heap.project(base, *index)
            }
            Some(TypedExprKind::BufGet { buf, index }) => {
                let base = self.place(*buf)?;
                let index = self.index(*index)?;
                self.index_address(*buf, base, index)
            }
            Some(TypedExprKind::Deref(inner)) => match self.value(*inner)? {
                EvalValue::Reference(address) => Ok(address),
                _ => Err(Diagnostic::new(
                    "compile-time-invalid-deref",
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
        let replacement = if let Some(operator) = operator {
            let current = self.heap.freeze(address)?;
            match operator {
                crate::ResolvedCompoundOperator::Callable { target, .. } => {
                    self.call(*target, &[current, replacement])?
                }
                _ => {
                    return Err(Diagnostic::new(
                        "compile-time-invalid-reassignment",
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

    fn condition_matches(&mut self, condition: &crate::typed::TypedCondition) -> EvalResult<bool> {
        Ok(match condition {
            crate::typed::TypedCondition::Is { subject, pattern } => {
                let value = self.complete(*subject)?;
                crate::analysis::pattern::pattern_matches_public(&pattern.value, &value)
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
        self.call_values(
            target,
            &args
                .iter()
                .cloned()
                .map(EvalValue::Frozen)
                .collect::<Vec<_>>(),
        )?
        .freeze()
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
            return Err(Diagnostic::new(
                "compile-time-unknown-function",
                format!("callable `{}` has no definition", target.0),
            ));
        };
        if bind.params.len() != args.len() {
            return Err(Diagnostic::new(
                "compile-time-arity-mismatch",
                "compile-time function call has the wrong number of arguments",
            ));
        }
        let mut frame = HashMap::new();
        for ((name, _), value) in bind.params.iter().zip(args) {
            frame.insert(*name, self.store_local(value.clone()));
        }
        let caller_locals = std::mem::replace(&mut self.locals, vec![frame]);
        let result = (|| match &bind.body {
            crate::typed::BindBody::Expr(expr) => self.value(*expr),
            crate::typed::BindBody::Body { exprs, ret } => {
                for expr in exprs {
                    let _ = self.eval(*expr)?;
                }
                ret.map(|expr| self.value(expr)).unwrap_or_else(|| {
                    Ok(EvalValue::Frozen(ConstValue::Record {
                        fields: Vec::new().into(),
                    }))
                })
            }
            crate::typed::BindBody::Unlowered => Err(Diagnostic::new(
                "compile-time-missing-body",
                "callable has no lowered body",
            )),
        })();
        self.locals = caller_locals;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_across_files_restore_the_callers_expression_arena() {
        use crate::transform::{PackageTransformOptions, TransformCtx, transform_package};
        use parser::cursor::TokenCursor;

        let files = [
            "Word is in 0 to 255\necho(value Word) Word: value\n",
            "forward(value Word) Word: echo(value)\n",
            "outer(value Word) Word: forward(value)\nlocal(value Word) Word: value\n",
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
                evaluator.call(crate::DefId(Intern::from_ref(name)), &[ConstValue::ONE]),
                Ok(ConstValue::ONE)
            );
        }
        assert_eq!(evaluator.program.file_id, typed[2].file_id);
        assert!(evaluator.locals.is_empty());
        let error = evaluator
            .call(crate::DefId(Intern::from_ref("echo")), &[])
            .unwrap_err();
        assert_eq!(error.code.slug(), "compile-time-arity-mismatch");
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
        let base = heap.allocate_sequence([ConstValue::ZERO, ConstValue::ONE]);
        assert_eq!(heap.load_offset(base, 1), Ok(&ConstValue::ONE));
        heap.store_offset(base, 1, ConstValue::ZERO).unwrap();
        assert_eq!(heap.load_offset(base, 1), Ok(&ConstValue::ZERO));
        assert!(heap.load_offset(base, 2).is_err());
    }
}
