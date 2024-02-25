#![allow(unused)]
use std::{env, fs, path::Path, process::exit};

use crate::{exit_status::ExitStatus, parse::Parser};

mod exit_status;
mod lex;
mod parse;
mod tests;
pub mod token;

// we are assuming that if you pass a file you want it run
pub fn ast(path: &str) -> parse::Module {
    let path = Path::new(&path);
    if !path.exists() {
        eprintln!("No such file or directory: {}", path.display());
        exit(ExitStatus::NoSuchFileOrDirectory.into())
    }

    let mut parser = Parser::new();
    parser.parse_module(path)
}

// we are assuming that if you pass a file you want it run
pub fn run(path: String) {
    let path = Path::new(&path);
    if !path.exists() {
        eprintln!("No such file or directory: {}", path.display());
        exit(ExitStatus::NoSuchFileOrDirectory.into())
    }

    let mut parser = Parser::new();
    let module = parser.parse_module(path);
    println!("{:#?}", module);
}

fn main() {
    if let Some(path) = env::args().nth(1) {
        run(path)
    } else {
        eprintln!("Failed to provide gin file")
    }
}
