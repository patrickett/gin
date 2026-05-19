use indexmap::IndexMap;
use internment::Intern;
use std::fmt;

use crate::TypeExpr;
use crate::expr::{Expr, Typed};
use crate::span::Spanned;
use crate::ty_state::TyState;
use crate::type_surface_mangle_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ParamConvention {
    /// Bare `name Type` — compiler infers from body whether the param is
    /// threaded (appears in return) or consumed (never returned).
    #[default]
    Inferred,
    /// `~name Type` — function consumes the parameter; it is not returned.
    Consume,
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
    /// Type annotation for this parameter, as expression-shaped type syntax.
    Tagged(Box<Spanned<Expr>>),
    Default(Box<Typed<Expr>>),
}

pub(crate) fn fmt_type_expr_surface(e: &TypeExpr, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match e {
        TypeExpr::Nominal(name, _) => write!(f, "{}", name.as_str()),
        TypeExpr::Qualified(path) => {
            write!(f, "{}", path.root.as_str())?;
            for seg in &path.segments {
                write!(f, ".{}", seg.as_str())?;
            }
            Ok(())
        }
        TypeExpr::Generic { name, params, .. } => {
            write!(f, "{}(", name.as_str())?;
            let mut first = true;
            for (k, v) in params.iter() {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                match v {
                    ParameterKind::Tagged(sp) => {
                        if let Some(te) = sp.value.as_type_expr() {
                            // Positional type arg (key equals mangled type name):
                            // just show the type. Named param (key differs): show key + type.
                            if type_surface_mangle_name(&te) == k.as_str() {
                                fmt_type_expr_surface(&te, f)?;
                            } else {
                                write!(f, "{} ", k.as_str())?;
                                fmt_type_expr_surface(&te, f)?;
                            }
                        } else {
                            write!(f, "{} <type>", k.as_str())?;
                        }
                    }
                    ParameterKind::Default(expr) => {
                        write!(f, "{}: {expr:?}", k.as_str())?;
                    }
                    ParameterKind::Generic => {
                        write!(f, "{}", k.as_str())?;
                    }
                }
            }
            write!(f, ")")
        }
        TypeExpr::Literal(..) => write!(f, "<type>"),
        TypeExpr::Pointer(inner) => {
            write!(f, "@")?;
            fmt_type_expr_surface(&inner.value, f)
        }
        TypeExpr::Ref { inner, mutable } => {
            if *mutable {
                write!(f, "mut ")?;
            } else {
                write!(f, "ref ")?;
            }
            fmt_type_expr_surface(&inner.value, f)
        }
        TypeExpr::Unit => write!(f, "()"),
    }
}

pub fn format_type_surface(e: &TypeExpr) -> String {
    struct Fmt<'a>(&'a TypeExpr);
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
                if let Some(te) = sp.value.as_type_expr() {
                    fmt_type_expr_surface(&te, f)
                } else {
                    write!(f, "<type>")
                }
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
