//! MLIR code generation using Melior.
//!
//! This module translates the Gin AST into MLIR IR using standard dialects:
//! - `func`: For function definitions and calls
//! - `arith`: For arithmetic operations
//! - `scf`: For structured control flow (loops, if/else)

mod codegen_error;
mod lower;

// Re-export the main entry point and error type
pub use self::codegen_error::CodegenError;
pub use self::lower::generate_mlir;
