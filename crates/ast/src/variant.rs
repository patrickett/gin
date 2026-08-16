//! Union variants inside `declare` — expression-shaped type syntax only.

use std::hash::{Hash, Hasher};

use crate::doc_comment::DocComment;
use crate::span::Spanned;
use crate::{Expr, Pattern};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variant {
    External {
        shape: Box<Spanned<Pattern>>,
        /// Optional `-> result_type` (e.g. `Nil -> Vec(x, 0)`).
        result_ty: Option<Box<Spanned<Expr>>>,
    },
    Local {
        doc_comment: Option<DocComment>,
        shape: Box<Spanned<Pattern>>,
        /// Optional `-> result_type` (e.g. `Cons -> Vec(x, n + 1)`).
        result_ty: Option<Box<Spanned<Expr>>>,
    },
}

impl Variant {
    pub fn shape(&self) -> &Spanned<Pattern> {
        match self {
            Variant::External { shape, .. } | Variant::Local { shape, .. } => shape,
        }
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        match self {
            Variant::External { .. } => None,
            Variant::Local { doc_comment, .. } => doc_comment.as_ref(),
        }
    }

    pub fn result_ty(&self) -> Option<&Spanned<Expr>> {
        match self {
            Variant::External { result_ty, .. } | Variant::Local { result_ty, .. } => {
                result_ty.as_ref().map(|b| b.as_ref())
            }
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.shape())
    }
}

impl Hash for Variant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::External { shape, .. } | Self::Local { shape, .. } => shape.hash(state),
        }
    }
}
