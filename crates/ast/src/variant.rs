//! Union variants inside `declare` — expression-shaped type syntax only.

use std::hash::{Hash, Hasher};

use crate::TypeExpr;
use crate::doc_comment::DocComment;
use crate::parameter::{ParameterKind, fmt_type_expr_surface};
use crate::span::Spanned;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variant {
    /// this comes from somewhere else its just one of the possible values
    /// holds its own doc comments
    External(Box<Spanned<TypeExpr>>),
    /// defined within the current declare
    Local {
        doc_comment: Option<DocComment>,
        shape: Box<Spanned<TypeExpr>>,
    },
}

impl Variant {
    pub fn shape(&self) -> &Spanned<TypeExpr> {
        match self {
            Variant::External(sp) => sp,
            Variant::Local { shape, .. } => shape,
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_variant_shape_surface(&self.shape().value, f)
    }
}

fn fmt_variant_shape_surface(e: &TypeExpr, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match e {
        TypeExpr::Generic { name, params, .. } => {
            write!(f, "{}(", name.as_str())?;
            let mut first = true;
            for (k, v) in params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                match v {
                    ParameterKind::Tagged(sp) => {
                        if let Some(te) = sp.value.as_type_expr() {
                            fmt_type_expr_surface(&te, f)?
                        }
                    }
                    ParameterKind::Generic => write!(f, "{}", k.as_str())?,
                    ParameterKind::Default(expr) => write!(f, "{}: {:?}", k.as_str(), expr)?,
                }
            }
            write!(f, ")")
        }
        TypeExpr::Nominal(name, _) => write!(f, "{}", name.as_str()),
        TypeExpr::Qualified(path) => {
            write!(f, "{}", path.root.as_str())?;
            for seg in &path.segments {
                write!(f, ".{}", seg.as_str())?;
            }
            Ok(())
        }
        TypeExpr::Literal(lit, _) => write!(f, "{lit}"),
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
