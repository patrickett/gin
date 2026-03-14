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
