#![allow(unused)]
use std::{env, fs, path::Path, process::exit};

use crate::{exit_status::ExitStatus, parse::Parser};

mod exit_status;
mod lex;
mod parse;
pub mod token;

fn main() {
    if let Some(path) = env::args().nth(1) {
        let path = Path::new(&path);
        if !path.exists() {
            eprintln!("No such file or directory: {}", path.display());
            exit(ExitStatus::NoSuchFileOrDirectory.into())
        }

        let mut parser = Parser::new();
        let module = parser.parse_module(path);
        println!("{:#?}", module);
    } else {
        eprintln!("Failed to provide gin file")
    }
}
