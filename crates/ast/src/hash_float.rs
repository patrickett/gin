//! A wrapper around `f64` that implements [`Hash`] and [`Eq`].
//!
//! `f64` does not implement `Hash` or `Eq` in std because NaN has many
//! representations. This wrapper provides both via `to_bits()` for [`Hash`]
//! while preserving IEEE [`PartialEq`] semantics (NaN != NaN).

use std::hash::Hash;

/// Wraps an `f64` with [`Hash`] and [`Eq`] support.
#[derive(Debug, Clone, Copy)]
pub struct HashFloat(pub f64);

impl PartialEq for HashFloat {
    fn eq(&self, other: &Self) -> bool {
        // IEEE comparison: NaN != NaN.
        self.0.eq(&other.0)
    }
}

impl Eq for HashFloat {}

impl Hash for HashFloat {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl std::fmt::Display for HashFloat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
