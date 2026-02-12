use crossbeam_channel::unbounded;
use ginc::database::{File, input_database::InputDatabase};
use ginc::frontend::parser::construct::FileAst;
use ginc::frontend::parser::parse;
use std::path::PathBuf;

pub fn parse_str(src: &str) -> FileAst {
    let (tx, _rx) = unbounded();
    let db = InputDatabase::new(tx);
    let file = File::new(&db, PathBuf::from(":memory:"), src.to_string());
    parse(&db, file)
}
