//! Formatting utilities for AST types.
//!
//! This crate provides functions for producing human-readable "surface"
//! representations of AST nodes — short summaries suitable for hover,
//! diagnostics, and other short-form display. Full-fidelity source
//! formatting lives in `ginfmt`.

pub mod declare;
pub mod type_expr;

#[cfg(test)]
extern crate self as ast_format;
