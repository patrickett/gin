//! Test helpers for ast crate tests.

use crossbeam_channel::unbounded;
use database::{File, InputDatabase};
use ast::{FileAst, parse_file};
use std::path::PathBuf;

/// Parse source code into an AST using an in-memory database.
///
/// This is a convenience helper for tests that need to parse source strings
/// without setting up a full database infrastructure.
#[allow(unused)]
pub fn parse_str(src: &str) -> FileAst {
    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from(":memory:"), src.to_string());
    parse_file(&db, file)
}

/// Parse source as a module (same as parse_str for now).
#[allow(unused)]
pub fn parse_module_str(src: &str) -> FileAst {
    parse_str(src)
}

/// Parse source as a script (same as parse_str for now).
#[allow(unused)]
pub fn parse_script_str(src: &str) -> FileAst {
    parse_str(src)
}
