//! Bounds helpers — re-exported from `ast::range_bounds`.
//!
//! The canonical implementations now live on [`Ty`] and [`DeclareValue`] in the `ast` crate.
//! This module re-exports [`InclusiveBounds`] for convenience.

pub use ast::range_bounds::InclusiveBounds;
