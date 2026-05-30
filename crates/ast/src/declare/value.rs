use crate::TypeExpr;
use crate::WhenExpr;
use crate::parameter::fmt_type_expr_surface;
use crate::prelude::*;
use crate::span::Spanned;
use i256::I256;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareValue {
    Alias(Box<Spanned<TypeExpr>>),
    Record(Parameters),
    Union { variants: Vec<Variant> },
    /// Type-level conditional: `PointerSize is when target.arch is … then …`.
    When(Box<WhenExpr>),
    Set(/* TODO */),
    Range(I256, I256),
    // DiceThrow is in 1...6 (element of range)
    InRange(I256, I256),
}

impl std::fmt::Display for DeclareValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alias(sp) => fmt_type_expr_surface(&sp.value, f),
            Self::When(_) => write!(f, "when …"),
            Self::Record(params) => {
                write!(f, "(")?;
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
            Self::Union { variants } => {
                let all_literal = variants
                    .iter()
                    .all(|v| matches!(v.shape().value, TypeExpr::Literal(..)));
                if all_literal && !variants.is_empty() {
                    write!(f, "{}", variants[0])?;
                    for v in &variants[1..] {
                        write!(f, "\n             or {v}")?;
                    }
                    return Ok(());
                }
                let mut first = true;
                for v in variants {
                    if !first {
                        write!(f, " or ")?;
                    }
                    first = false;
                    write!(f, "{v}")?;
                }
                Ok(())
            }
            Self::Set() => write!(f, "set"),
            Self::Range(start, end) => write!(f, "{start}...{end}"),
            Self::InRange(start, end) => write!(f, "in {start}...{end}"),
        }
    }
}

impl Hash for DeclareValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Alias(sp) => sp.hash(state),
            Self::Record(params) => {
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
            Self::When(w) => w.hash(state),
            Self::Union { variants } => {
                for variant in variants {
                    variant.hash(state);
                }
            }
            Self::Set() => {}
            Self::Range(start, end) | Self::InRange(start, end) => {
                start.hash(state);
                end.hash(state);
            }
        }
    }
}
