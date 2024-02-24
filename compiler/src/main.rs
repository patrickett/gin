#![allow(unused)]
use std::{env, fs, path::Path, process::exit};

use crate::{exit_status::ExitStatus, parse::Parser};

mod exit_status;
mod lex;
mod parse;

fn main() {
    if let Some(path) = env::args().nth(1) {
        let path = Path::new(&path);
        if !path.exists() {
            eprintln!("No such file or directory: {}", path.display());
            exit(ExitStatus::NoSuchFileOrDirectory.into())
        }

        if let Ok(source_file) = fs::read_to_string(path) {
            let file_name = path
                .file_stem()
                .expect("Failed to read file name")
                .to_str()
                .expect("Failed to convert file name to str");

            let mut lexer = lex::Lexer::new();
            let tokens = lexer.lex(&source_file);
            // a file exports a default empty module with the name of the file

            let mut parser = Parser::new();
            let ast = parser.parse(tokens);

            println!("ast: \n{:#?}", ast);
        } else {
            eprintln!("Unable to read contents of file: {}", path.display());
            // TODO: better exit code?
            exit(ExitStatus::NoSuchFileOrDirectory.into())
        }
    } else {
        eprintln!("Failed to provide gin file")
    }
}
