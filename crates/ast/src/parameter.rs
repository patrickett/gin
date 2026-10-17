use indexmap::IndexMap;
use internment::Intern;

use crate::GroupPath;
use crate::ProvidedTrait;
use crate::expr::{Expr, Typed};
use crate::span::SpanId;
use crate::span::Spanned;
use crate::ty_state::TyState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParamConvention {
    #[default]
    Terminal,
    Own,
    Observe,
    Mutate,
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
    pub constraints: Vec<ProvidedTrait>,
    pub range_pattern: Option<RangeParameterPattern>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RangeParameterPattern {
    pub start: Intern<String>,
    pub end: Intern<String>,
    pub end_span: SpanId,
}

impl Parameter {
    pub fn new(name_span: SpanId, kind: ParameterKind) -> Self {
        Self {
            kind,
            name_span,
            constraints: Vec::new(),
            range_pattern: None,
        }
    }

    pub fn with_constraints(mut self, constraints: Vec<ProvidedTrait>) -> Self {
        self.constraints = constraints;
        self
    }

    pub fn constraints_surface(&self) -> String {
        self.constraints
            .iter()
            .map(|constraint| constraint.trait_name.as_str())
            .collect::<Vec<_>>()
            .join(" and ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    Generic,
    Proposition(Box<crate::ProofProposition>),
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
            ParameterKind::Proposition(predicate) => write!(f, " is {predicate}"),
            ParameterKind::Tagged(sp) => {
                write!(f, " {:?}", sp.value)
            }
            ParameterKind::Default(expr) => write!(f, ": {:?}", expr),
            ParameterKind::ValueParam { ty } => write!(f, " {:?} (value)", ty.value),
            ParameterKind::Inferred { ty } => write!(f, " {:?}: ?", ty.value),
        }
    }
}

pub type Parameters = IndexMap<Intern<String>, Parameter>;
