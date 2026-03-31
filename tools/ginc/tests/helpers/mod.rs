use crossbeam_channel::unbounded;
use ginc::database::{File, input_database::InputDatabase};
use ginc::ast::FileAst;
use ginc::parse::query::parse;
use std::path::PathBuf;

#[allow(unused)]
pub fn parse_str(src: &str) -> FileAst {
    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from(":memory:"), src.to_string());
    parse(&db, file)
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
