//! Runs the new checker's recorded bodies directly.
//!
//! Loads checked modules into [`Program`] and exposes the shared [`Interpreter`].
//! Functions without recorded bodies fail with [`Error::NoBody`]; evaluation never
//! falls back to native compilation.

mod program;

// The evaluator itself is shared with `typecheck`.
pub use program::{LoadError, Program};
pub use typecheck::run::{Error, Interpreter, Limits, Outcome, RtType, Tag, Value};

/// Run `work` on a thread with a stack deep enough for the interpreter's recursion (one Rust
/// frame per body node on the longest dependency chain). Test and tool code should wrap whole
/// sessions in this; the library itself never spawns threads.
pub fn with_big_stack<R: Send>(work: impl FnOnce() -> R + Send) -> R {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(512 * 1024 * 1024)
            .spawn_scoped(scope, work)
            .expect("the interpreter thread starts")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}
