//! `ginlsp`: LSP over stdio.

use std::io::{BufReader, stdin, stdout};
use std::process::ExitCode;

fn main() -> ExitCode {
    // The checker recurses over bodies; give the server a deep stack.
    let code = interpreter::with_big_stack(|| {
        let input = BufReader::new(stdin().lock());
        ginlsp::server::serve(input, stdout().lock())
    });
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
