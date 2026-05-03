//! Union variants inside `declare` — expression-shaped type syntax only.

use std::hash::{Hash, Hasher};

use crate::doc_comment::DocComment;
use crate::expr::Expr;
use crate::parameter::fmt_type_expr_surface;
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variant {
    /// this comes from somewhere else its just one of the possible values
    /// holds its own doc comments
    External(Box<Spanned<Expr>>),
    /// defined within the current declare
    Local {
        doc_comment: Option<DocComment>,
        shape: Box<Spanned<Expr>>,
    },
}

impl Variant {
    pub fn shape(&self) -> &Spanned<Expr> {
        match self {
            Variant::External(sp) => sp,
            Variant::Local { shape, .. } => shape,
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_variant_shape_surface(&self.shape().0, f)
    }
}

fn fmt_variant_shape_surface(e: &Expr, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match e {
        Expr::TypeGeneric { name, params, .. } => {
            write!(f, "{}(", name.as_str())?;
            let mut first = true;
            for (k, v) in params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}{v}", k.as_str())?;
            }
            write!(f, ")")
        }
        _ => fmt_type_expr_surface(e, f),
    }
}

impl Hash for Variant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::External(sp) => sp.hash(state),
            Self::Local { doc_comment, shape } => {
                doc_comment.hash(state);
                shape.hash(state);
            }
        }
    }
}
