//! A machine model whose encodings are Gin.
//!
//! * An instruction is a value ([`Instruction`]), not a name the compiler knows.
//! * Its meaning is one function, [`semantics::execute`], over a [`semantics::Machine`].
//!   Run it on [`concrete::Concrete`] to emulate, or on [`abstract_machine::Abstract`] to
//!   check: register effects, clobbers, bounds and callee-saved registers are derived from
//!   the same semantics, so there is no second, assembly-specific checker.
//! * Its encoding is Gin (`gin/arm64.gin`), checked by the Gin checker and evaluated by the
//!   compile-time evaluator when the fields are known ([`encode::Arm64`]).
//!
//! * The machine itself is Gin too (`gin_semantics`): registers are linear records, `Word`
//!   wraps, and each `execute_*` function is run by the compile-time evaluator. The Rust
//!   `execute` is the cross-check.
//!
// TODO(instruction-model): replace the duplicated Rust/Gin semantic definitions with one checked
// Gin model. Cross-check native execution independently before trusting inferred effects.
// Encoder and rule-table experiments here must not become a second production selection path.

pub mod abstract_machine;
pub mod concrete;
pub mod emit;
pub mod encode;
pub mod gin_semantics;
pub mod instruction;
pub mod lower;
pub mod program;
pub mod rules;
pub mod semantics;

pub use instruction::{Instruction, Reg};
pub use program::{Data, check, check_with, run};
pub use semantics::ModelError;
