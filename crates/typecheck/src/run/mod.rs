//! The evaluator that runs recorded bodies with the full semantics of the language: lazy over
//! regions, generics as templates with a type environment, `while`, holes, effects forced then
//! refused, and every integer re-checked against the domain the checker recorded for it.
//!
//! It lives beside the checker's compile-time fold (`source_check::eval`, the eager strategy
//! with the checker's budgets) and shares everything that is not control structure with it:
//! `apply`/`exact_binary` (arithmetic), `wrap_into`, and `regions` (which nodes run only when a
//! region does). `interpreter` wraps it with loading, rendering and a command line.

mod error;
mod interp;
mod types;
mod value;

pub use error::Error;
pub use interp::{Interpreter, Limits, Outcome};
pub use types::RtType;
pub use value::{Tag, Value};
