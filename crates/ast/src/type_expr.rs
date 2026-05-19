use crate::expr::Literal;
use crate::parameter::ParameterKind;
use crate::path::ModPath;
use crate::span::{SpanId, Spanned};
use internment::Intern;

/// A type expression — the right-hand side of a type annotation.
///
/// This replaces the old `Expr::TypeNominal`, `Expr::TypeQualified`, and
/// `Expr::TypeGeneric` variants so that type expressions are a distinct AST
/// node from value expressions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeExpr {
    /// Bare `Tag` (e.g. `Str` in `(x Str)`).
    Nominal(Intern<String>, SpanId),
    /// Qualified path `Tag.Tag…`.
    Qualified(Spanned<ModPath>),
    /// `Tag(...)` with generic / named parameters.
    Generic {
        name: Intern<String>,
        params: Vec<(Intern<String>, ParameterKind)>,
        span: SpanId,
    },
    /// Literal value in type position (e.g. union variant like `X0 is 'x0'`).
    Literal(Literal, SpanId),
    /// Pointer to another type (e.g. `@x` — raw pointer to `x`).
    Pointer(Box<Spanned<TypeExpr>>),
    /// Reference type: `ref T` or `mut T`.
    Ref {
        inner: Box<Spanned<TypeExpr>>,
        mutable: bool,
    },
    /// The unit type `()`.
    Unit,
}
