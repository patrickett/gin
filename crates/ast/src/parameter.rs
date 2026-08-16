use indexmap::IndexMap;
use internment::Intern;

use crate::GroupPath;
use crate::expr::{Expr, Typed};
use crate::span::SpanId;
use crate::span::Spanned;
use crate::ty_state::TyState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParamConvention {
    #[default]
    Own,
    Observe,
    Mutate,
    Consume,
}

/// Compatibility view derived from grouped parameter reference types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupParam {
    pub path: GroupPath,
    pub ty_name: Intern<String>,
    pub mutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamSlot {
    pub ty: TyState,
    /// Default value expression, e.g. `x: 4` in `func(x: 4)`.
    pub default: Option<Typed<Expr>>,
    pub convention: ParamConvention,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Parameter {
    pub kind: ParameterKind,
    /// Span for the declaration identifier.
    pub name_span: SpanId,
}

impl Parameter {
    pub fn new(name_span: SpanId, kind: ParameterKind) -> Self {
        Self { kind, name_span }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    Generic,
    /// Type annotation for this parameter (`name Type` with no value).
    Tagged(Box<Spanned<Expr>>),
    Default(Box<Typed<Expr>>),
    /// Value parameter in a type declaration, e.g. `n Nat` in `Vector(x, n Nat)`.
    /// The wrapped type is the value's type (e.g. `Nat`).
    ValueParam {
        ty: Box<Spanned<Expr>>,
    },
    Inferred {
        ty: Box<Spanned<Expr>>,
    },
}

impl std::fmt::Display for ParameterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterKind::Generic => Ok(()),
            ParameterKind::Tagged(sp) => {
                write!(f, " {:?}", sp.value)
            }
            ParameterKind::Default(expr) => write!(f, ": {:?}", expr),
            ParameterKind::ValueParam { ty } => write!(f, " {:?} (value)", ty.value),
            ParameterKind::Inferred { ty } => write!(f, " {:?}: ?", ty.value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamInfo {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Box<Spanned<Expr>>),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Box<Typed<Expr>>),
}

pub type Parameters = IndexMap<Intern<String>, Parameter>;
