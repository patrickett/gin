use indexmap::IndexMap;
use internment::Intern;

use crate::TypeExpr;
use crate::expr::{Expr, Typed};
use crate::span::Spanned;
use crate::ty_state::TyState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParamConvention {
    /// Bare `name Type` — compiler infers from body whether the param is
    /// threaded (appears in return) or consumed (never returned).
    #[default]
    Inferred,
    /// `ref name Type` — immutable reference parameter.
    /// The parameter is borrowed, not consumed.
    Ref(bool),
    /// `eat name Type` — explicit consume parameter.
    Eat,
}

/// A group parameter on a function: `[mut r T]` or `[r T]`.
/// Declares that multiple params may alias because they belong to the same group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupParam {
    pub name: Intern<String>,
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
pub enum ParameterKind {
    Generic,
    /// Type annotation for this parameter (`name Type` with no value).
    Tagged(Box<Spanned<TypeExpr>>),
    Default(Box<Typed<Expr>>),
}

impl std::fmt::Display for ParameterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterKind::Generic => Ok(()),
            ParameterKind::Tagged(sp) => {
                write!(f, " {:?}", sp.value)
            }
            ParameterKind::Default(expr) => write!(f, ": {:?}", expr),
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

// TODO: store a SpanId per parameter so LSP rename/go-to-def/find-references
// can resolve parameter declarations and body usages precisely.
// e.g. IndexMap<Intern<String>, (ParameterKind, SpanId)>
pub type Parameters = IndexMap<Intern<String>, ParameterKind>;
