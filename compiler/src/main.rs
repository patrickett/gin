use std::path::Path;

use clap::*;
use ngin::{
    parser::lexer::{token::Token, Lexer},
    source_file::SourceFile,
    Ngin,
};
pub mod ngin;

mod tests;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Path to the .gin file
    file_path: Option<String>,

    /// Print abstract syntax tree for the provided file
    #[arg(short, long)]
    debug: bool,
    /// Print lexed tokens for the provided file
    #[arg(short, long)]
    tokens: bool,
}

fn main() {
    let args = Args::parse();
    match args.file_path {
        Some(path) => {
            let mut runtime = Ngin::new();
            let body = runtime.include(&path);
            if args.tokens {
                let mut lexer = Lexer::new();
                let path = Path::new(&path);
                let source_file = SourceFile::new(path);
                lexer.set_content(&source_file);
                let tokens: Vec<Token> = lexer.collect();
                println!("{:#?}", tokens);
                // runtime.tokens(&body)
            }
            if args.debug {
                println!("{:#?}", body);
            }

            if !args.debug && !args.tokens {
                runtime.execute(&body);
            }
        }
        None => println!("starting repl"),
    }
}
