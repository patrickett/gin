use crate::prelude::*;
use std::{
    hash::{Hash, Hasher},
    ops::Range,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareValue {
    Alias(Tag),
    Record(Parameters),
    Union { variants: Vec<Variant> },
    Set(/* TODO */),
    Range(Range<i64>),
    // DiceThrow is in 1...6 (element of range)
    InRange(Range<i64>),
}

impl std::fmt::Display for DeclareValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alias(tag) => write!(f, "{tag}"),
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
            Self::Range(r) => write!(f, "{}...{}", r.start, r.end),
            Self::InRange(r) => write!(f, "in {}...{}", r.start, r.end),
        }
    }
}

impl Hash for DeclareValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Alias(tag) => tag.hash(state),
            Self::Record(params) => {
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
            Self::Union { variants } => {
                for variant in variants {
                    variant.hash(state);
                }
            }
            Self::Set() => {}
            Self::Range(r) | Self::InRange(r) => {
                r.start.hash(state);
                r.end.hash(state);
            }
        }
    }
}
