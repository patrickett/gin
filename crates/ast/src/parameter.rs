use indexmap::IndexMap;
use internment::Intern;
use std::fmt;

use crate::expr::Expr;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    Generic,
    /// Type annotation for this parameter, as expression-shaped type syntax.
    Tagged(Box<Spanned<Expr>>),
    Default(Spanned<Expr>),
}

pub(crate) fn fmt_type_expr_surface(e: &Expr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match e {
        Expr::TypeNominal(name, _) => write!(f, "{}", name.as_str()),
        Expr::TypeQualified(path) => {
            write!(f, "{}", path.root.as_str())?;
            for seg in &path.segments {
                write!(f, ".{}", seg.as_str())?;
            }
            Ok(())
        }
        Expr::TypeGeneric { name, params, .. } => {
            write!(f, "{}[", name.as_str())?;
            let mut first = true;
            for (k, v) in params.iter() {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}{v}", k.as_str())?;
            }
            write!(f, "]")
        }
        _ => write!(f, "<type>"),
    }
}

/// Pretty-print a type-surface [`Expr`] (`TypeNominal` / `TypeQualified` / `TypeGeneric`).
pub fn format_type_surface(e: &Expr) -> String {
    struct Fmt<'a>(&'a Expr);
    impl fmt::Display for Fmt<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt_type_expr_surface(self.0, f)
        }
    }
    Fmt(e).to_string()
}

impl std::fmt::Display for ParameterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterKind::Generic => Ok(()),
            ParameterKind::Tagged(sp) => {
                write!(f, " ")?;
                fmt_type_expr_surface(&sp.0, f)
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
    Default(Spanned<Expr>),
}

// TODO: store a SpanId per parameter so LSP rename/go-to-def/find-references
// can resolve parameter declarations and body usages precisely.
// e.g. IndexMap<Intern<String>, (ParameterKind, SpanId)>
pub type Parameters = IndexMap<Intern<String>, ParameterKind>;
