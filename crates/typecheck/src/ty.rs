//! Type representation — re-exported from `ast` for convenience.
//!
//! `Ty` is defined in `ast` because parse-level types like `TyState`
//! and `TypeExpr` reference it. This module re-exports the `ast::ty`
//! types so that consumers of `typecheck` can use `typecheck::ty::Ty`.

pub use ast::ty::*;
