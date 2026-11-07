//! Native backend for the new stack: MLIR context, MLIR text to object, and linking.
//! Independent of the legacy `codegen` crate.

mod emit;
mod process;
mod stack;

pub use emit::{NativeCompiler, Profile};
