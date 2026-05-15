//! Layout / size computation utilities.
//! Re-exported from `ast` since they're tightly coupled to the `Ty` type.

#[allow(unused_imports)]
pub use ast::ty::{ty_alignment, ty_byte_size_static, ty_union_discriminant_size};
