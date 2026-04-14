use indexmap::IndexMap;
use internment::Intern;

use crate::expr::Expr;
use crate::span::Spanned;
use crate::tag::Tag;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    Generic,
    Tagged(Tag),
    Default(Spanned<Expr>),
}

impl std::fmt::Display for ParameterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterKind::Generic => Ok(()),
            ParameterKind::Tagged(tag) => write!(f, " {}", tag),
            ParameterKind::Default(expr) => write!(f, ": {:?}", expr),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamInfo {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Tag),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Spanned<Expr>),
}

pub type Parameters = IndexMap<Intern<String>, ParameterKind>;
