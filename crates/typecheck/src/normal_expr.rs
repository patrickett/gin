//! Checked integer simplification for symbolic dependent expressions.

use ast::type_decl::TypeNameExt;
use ast::{ConstValue, NormalExpr, Ty, TyArg};
use i256::I256;

pub trait Normalize: Sized {
    fn normalize(&self) -> Self;

    fn normalize_to_value(&self) -> Option<ConstValue> {
        let _ = self;
        None
    }
}

/// How a size expression's callable leaf is named.
#[derive(Clone, Copy)]
pub(crate) enum SizeCallName {
    /// The callable's mangled name (`call.mangled_name()`).
    Mangled,
    /// Only the root path segment.
    PathRoot,
}

/// Folds a size-position expression (`N`, `name`, `a + b`, …) into a symbolic
/// [`NormalExpr`], with call naming and lowercase-tag acceptance supplied by the
/// callers that use this for inference, type surfaces, and allocation sizes.
pub(crate) fn size_expr_to_normal(
    expr: &ast::Expr,
    call_name: SizeCallName,
    lowercase_tag: bool,
) -> Option<NormalExpr> {
    match expr {
        ast::Expr::Lit(ast::Literal::Int(n)) => Some(NormalExpr::from(*n)),
        ast::Expr::AnonymousTag(name) => {
            let lowercase = name.as_str().starts_with_lowercase();
            (lowercase_tag && lowercase).then_some(NormalExpr::Var(*name))
        }
        ast::Expr::FnCall(call) if call.args.is_none() => {
            let name = match call_name {
                SizeCallName::Mangled => call.mangled_name(),
                SizeCallName::PathRoot => call.path.value.root,
            };
            Some(NormalExpr::Var(name))
        }
        ast::Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        ast::Expr::Binary(binary) => {
            let lhs = size_expr_to_normal(&binary.lhs.value, call_name, lowercase_tag)?;
            let rhs = size_expr_to_normal(&binary.rhs.value, call_name, lowercase_tag)?;
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

#[derive(Clone, Copy)]
enum Operation {
    Add,
    Subtract,
    Multiply,
}

impl Operation {
    fn expression(self, left: NormalExpr, right: NormalExpr) -> NormalExpr {
        let left = Box::new(left);
        let right = Box::new(right);
        match self {
            Self::Add => NormalExpr::Add(left, right),
            Self::Subtract => NormalExpr::Sub(left, right),
            Self::Multiply => NormalExpr::Mul(left, right),
        }
    }

    fn checked(self, left: I256, right: I256) -> Option<I256> {
        match self {
            Self::Add => left.checked_add(right),
            Self::Subtract => left.checked_sub(right),
            Self::Multiply => left.checked_mul(right),
        }
    }
}

enum Task<'a> {
    Visit(&'a NormalExpr),
    Apply(Operation),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IntegerEvaluation {
    Total,
    Partial,
    Unsupported,
}

struct Normalized {
    expression: NormalExpr,
    evaluation: IntegerEvaluation,
}

impl Normalized {
    fn leaf(expression: &NormalExpr) -> Self {
        let evaluation = match expression {
            NormalExpr::Value(ConstValue::Int(_))
            | NormalExpr::Var(_)
            | NormalExpr::Inferred(_) => IntegerEvaluation::Total,
            NormalExpr::TargetQuery { .. } => IntegerEvaluation::Partial,
            NormalExpr::Value(_) => IntegerEvaluation::Unsupported,
            NormalExpr::Add(_, _) | NormalExpr::Sub(_, _) | NormalExpr::Mul(_, _) => unreachable!(),
        };
        Self {
            expression: expression.clone(),
            evaluation,
        }
    }
}

impl Normalize for NormalExpr {
    fn normalize(&self) -> NormalExpr {
        let mut pending = vec![Task::Visit(self)];
        let mut normalized = Vec::new();
        while let Some(task) = pending.pop() {
            match task {
                Task::Visit(expression) => {
                    let (operation, left, right) = match expression {
                        NormalExpr::Add(left, right) => (Operation::Add, left, right),
                        NormalExpr::Sub(left, right) => (Operation::Subtract, left, right),
                        NormalExpr::Mul(left, right) => (Operation::Multiply, left, right),
                        _ => {
                            normalized.push(Normalized::leaf(expression));
                            continue;
                        }
                    };
                    pending.extend([
                        Task::Apply(operation),
                        Task::Visit(right),
                        Task::Visit(left),
                    ]);
                }
                Task::Apply(operation) => {
                    let right = normalized.pop().expect("normalized right operand");
                    let left = normalized.pop().expect("normalized left operand");
                    normalized.push(normalize_binary(operation, left, right));
                }
            }
        }
        assert_eq!(normalized.len(), 1);
        normalized.pop().expect("normalized expression").expression
    }

    fn normalize_to_value(&self) -> Option<ConstValue> {
        match self.normalize() {
            NormalExpr::Value(value) => Some(value),
            _ => None,
        }
    }
}

fn normalize_binary(operation: Operation, left: Normalized, right: Normalized) -> Normalized {
    if left.evaluation == IntegerEvaluation::Unsupported
        || right.evaluation == IntegerEvaluation::Unsupported
    {
        return Normalized {
            expression: operation.expression(left.expression, right.expression),
            evaluation: IntegerEvaluation::Unsupported,
        };
    }
    if let (NormalExpr::Value(ConstValue::Int(a)), NormalExpr::Value(ConstValue::Int(b))) =
        (&left.expression, &right.expression)
    {
        if let Some(value) = operation.checked(*a, *b) {
            return Normalized {
                expression: value.into(),
                evaluation: IntegerEvaluation::Total,
            };
        }
    }
    match operation {
        Operation::Add => {
            if left.expression.is_zero() {
                return right;
            }
            if right.expression.is_zero() {
                return left;
            }
        }
        Operation::Subtract => {
            if right.expression.is_zero() {
                return left;
            }
            if left.evaluation == IntegerEvaluation::Total && left.expression == right.expression {
                return Normalized::leaf(&0_i128.into());
            }
        }
        Operation::Multiply => {
            if left.expression.is_one() {
                return right;
            }
            if right.expression.is_one() {
                return left;
            }
            if left.expression.is_zero() && right.evaluation == IntegerEvaluation::Total {
                return left;
            }
            if right.expression.is_zero() && left.evaluation == IntegerEvaluation::Total {
                return right;
            }
        }
    }
    let expression = match operation {
        Operation::Add => normalize_addition(left.expression, right.expression),
        _ => operation.expression(left.expression, right.expression),
    };
    Normalized {
        expression,
        evaluation: IntegerEvaluation::Partial,
    }
}

fn normalize_addition(left: NormalExpr, right: NormalExpr) -> NormalExpr {
    let merged = match (&left, &right) {
        (NormalExpr::Value(ConstValue::Int(offset)), NormalExpr::Add(left, right))
        | (NormalExpr::Add(left, right), NormalExpr::Value(ConstValue::Int(offset))) => {
            addition_offset(*offset, left, right)
        }
        _ => None,
    };
    if let Some((offset, first)) = merged {
        let ((NormalExpr::Value(_), NormalExpr::Add(left, right))
        | (NormalExpr::Add(left, right), NormalExpr::Value(_))) = (left, right)
        else {
            unreachable!()
        };
        return if first {
            NormalExpr::Add(Box::new(offset.into()), right)
        } else {
            NormalExpr::Add(left, Box::new(offset.into()))
        };
    }
    NormalExpr::Add(Box::new(left), Box::new(right))
}

fn addition_offset(offset: I256, left: &NormalExpr, right: &NormalExpr) -> Option<(I256, bool)> {
    let (constant, first) = match (left, right) {
        (NormalExpr::Value(ConstValue::Int(constant)), _) => (*constant, true),
        (_, NormalExpr::Value(ConstValue::Int(constant))) => (*constant, false),
        _ => return None,
    };
    if offset.is_negative() != constant.is_negative() {
        return None;
    }
    Some((offset.checked_add(constant)?, first))
}

pub(crate) fn normalize_type_constants(mut normalized: Ty) -> Ty {
    let mut pending = vec![&mut normalized];
    while let Some(ty) = pending.pop() {
        match ty {
            Ty::Named { instance, .. } => {
                normalize_type_arguments(&mut instance.arguments, &mut pending);
            }
            Ty::Record {
                fields,
                resolved_params,
                ..
            } => {
                pending.extend(fields.iter_mut().map(|(_, ty)| ty.as_mut()));
                if let Some(arguments) = resolved_params {
                    normalize_type_arguments(arguments, &mut pending);
                }
            }
            Ty::Union {
                variants,
                resolved_params,
                ..
            } => {
                for variant in variants {
                    pending.extend(variant.fields.iter_mut().map(|(_, ty)| ty.as_mut()));
                    if let Some(ty) = &mut variant.result_ty {
                        pending.push(ty);
                    }
                }
                if let Some(arguments) = resolved_params {
                    normalize_type_arguments(arguments, &mut pending);
                }
            }
            Ty::Array { elem, size } => {
                *size = size.normalize();
                pending.push(elem);
            }
            Ty::UnresolvedSequence { element, length } => {
                *length = length.normalize();
                if let Some(ty) = element {
                    pending.push(ty);
                }
            }
            Ty::Ref { inner, .. }
            | Ty::Ptr { inner }
            | Ty::OwnedStorage { inner }
            | Ty::Address { pointee: inner, .. } => pending.push(inner),
            Ty::Sequence(elements) => {
                pending.extend(elements.iter_mut().map(|element| element.ty.as_mut()));
            }
            Ty::Opaque(_)
            | Ty::AnonymousInteger { .. }
            | Ty::Float { .. }
            | Ty::Unit
            | Ty::Literal(_)
            | Ty::Proposition(_)
            | Ty::ResultFamily { .. }
            | Ty::UnresolvedLiteral(_) => {}
        }
    }
    normalized
}

fn normalize_type_arguments<'a>(
    arguments: &'a mut [(internment::Intern<String>, TyArg)],
    pending: &mut Vec<&'a mut Ty>,
) {
    for (_, argument) in arguments {
        match argument {
            TyArg::Type(ty) => pending.push(ty),
            TyArg::Const(expression) => *expression = expression.normalize(),
        }
    }
}
