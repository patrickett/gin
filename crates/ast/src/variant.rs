//! Union variants inside `declare` — expression-shaped type syntax only.

use std::hash::{Hash, Hasher};

use crate::TypeExpr;
use crate::doc_comment::DocComment;
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

    pub fn doc_comment(&self) -> Option<&DocComment> {
        match self {
            Variant::External(_) => None,
            Variant::Local { doc_comment, .. } => doc_comment.as_ref(),
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
            Self::External(sp) => sp.hash(state),
            Self::Local { doc_comment, shape } => {
                doc_comment.hash(state);
                shape.hash(state);
            }
        }
    }
}
