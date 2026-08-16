//! Symbolic compile-time constant expressions.
//!
//! `NormalExpr` represents unevaluated constant expressions (e.g. `n + m`, `2 * n`)
//! in contrast with [`ConstValue`](crate::ConstValue) which holds already-evaluated constants.

use crate::ConstValue;
use internment::Intern;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinderOwner {
    Definition(Intern<String>),
    Expression(u32),
    Parameter {
        definition: Intern<String>,
        slot: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BinderId {
    pub file: u32,
    pub owner: BinderOwner,
}

impl BinderId {
    pub fn new(file: u32, owner: BinderOwner) -> Self {
        Self { file, owner }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DependentArgId {
    pub binder: BinderId,
    pub slot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetQueryKind {
    Size,
    Alignment,
    IndexBits,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeReference {
    Unit,
    Nominal(Intern<String>),
    Application {
        name: Intern<String>,
        arguments: Vec<TypeReference>,
    },
    Pointer(Box<TypeReference>),
}

impl TypeReference {
    pub fn head_name(&self) -> Option<Intern<String>> {
        match self {
            Self::Nominal(name) | Self::Application { name, .. } => Some(*name),
            Self::Unit | Self::Pointer(_) => None,
        }
    }
}

impl DependentArgId {
    pub fn new(binder: BinderId, slot: u32) -> Self {
        Self { binder, slot }
    }
}

/// A symbolic compile-time constant expression — unevaluated, in contrast with
/// [`ConstValue`]. Evaluation/normalization is handled by a separate pass.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NormalExpr {
    Value(ConstValue),
    Var(Intern<String>),
    Inferred(DependentArgId),
    Add(Box<NormalExpr>, Box<NormalExpr>),
    Sub(Box<NormalExpr>, Box<NormalExpr>),
    Mul(Box<NormalExpr>, Box<NormalExpr>),
    TargetQuery {
        kind: TargetQueryKind,
        operand: TypeReference,
    },
}

impl NormalExpr {
    pub fn is_zero(&self) -> bool {
        *self == ConstValue::ZERO
    }

    pub fn is_one(&self) -> bool {
        *self == ConstValue::ONE
    }

    pub fn as_const_int(&self) -> Option<i128> {
        match self {
            NormalExpr::Value(ConstValue::Int(n))
                if *n >= i256::I256::from(i128::MIN) && *n <= i256::I256::from(i128::MAX) =>
            {
                Some(n.as_i128())
            }
            _ => None,
        }
    }
}

impl From<i128> for NormalExpr {
    fn from(n: i128) -> Self {
        NormalExpr::Value(ConstValue::Int(i256::I256::from(n)))
    }
}

impl From<i256::I256> for NormalExpr {
    fn from(value: i256::I256) -> Self {
        Self::Value(ConstValue::Int(value))
    }
}

impl TryFrom<&NormalExpr> for usize {
    type Error = ();

    fn try_from(expr: &NormalExpr) -> Result<usize, ()> {
        expr.as_const_int()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(())
    }
}

impl PartialEq<ConstValue> for NormalExpr {
    fn eq(&self, other: &ConstValue) -> bool {
        matches!(self, NormalExpr::Value(v) if v == other)
    }
}

impl fmt::Display for NormalExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NormalExpr::Value(v) => match v {
                ConstValue::Int(i) => write!(f, "{i}"),
                ConstValue::Float(hf) => write!(f, "{hf}"),
                ConstValue::String(s) => write!(f, "\"{s}\""),
                ConstValue::Tag {
                    name,
                    qual_path,
                    args,
                } => {
                    if let Some(qp) = qual_path {
                        write!(f, "{qp}.{}", name.as_str())?;
                    } else {
                        write!(f, "{}", name.as_str())?;
                    }
                    if !args.is_empty() {
                        write!(f, "(")?;
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                write!(f, ", ")?;
                            }
                            write!(f, "{}", NormalExpr::Value(arg.clone()))?;
                        }
                        write!(f, ")")?;
                    }
                    Ok(())
                }
                ConstValue::ResultAlternative { label, .. } => {
                    write!(f, "{}", label.as_str())
                }
                ConstValue::Record { fields } => {
                    write!(f, "{{ ")?;
                    for (i, (name, val)) in fields.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}: {}", name.as_str(), NormalExpr::Value(val.clone()))?;
                    }
                    write!(f, " }}")
                }
                ConstValue::List(items) => {
                    write!(f, "[")?;
                    for (i, item) in items.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", NormalExpr::Value(item.clone()))?;
                    }
                    write!(f, "]")
                }
            },
            NormalExpr::Var(name) => write!(f, "{}", name.as_str()),
            NormalExpr::Inferred(_) => write!(f, "?"),
            NormalExpr::Add(l, r) => write!(f, "({l} + {r})"),
            NormalExpr::Sub(l, r) => write!(f, "({l} - {r})"),
            NormalExpr::Mul(l, r) => write!(f, "({l} * {r})"),
            NormalExpr::TargetQuery { kind, operand } => {
                write!(f, "#{kind}({operand})")
            }
        }
    }
}

impl fmt::Display for TargetQueryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size => f.write_str("size"),
            Self::Alignment => f.write_str("alignment"),
            Self::IndexBits => f.write_str("index_bits"),
        }
    }
}

impl fmt::Display for TypeReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => f.write_str("()"),
            Self::Nominal(name) => f.write_str(name.as_str()),
            Self::Application { name, arguments } => {
                write!(f, "{}(", name.as_str())?;
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{argument}")?;
                }
                f.write_str(")")
            }
            Self::Pointer(inner) => write!(f, "@{inner}"),
        }
    }
}
