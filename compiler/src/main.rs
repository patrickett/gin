use std::path::Path;

use crate::ngin::{compiler_error::CompilerError, parser::ast::Node};
use clap::*;
use ngin::{
    parser::lexer::{token::Token, Lexer},
    source_file::SourceFile,
    validator::validate,
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

    /// Print the compiled llvm ir
    #[arg(short, long)]
    compile: bool,
}

fn handle_error<T>(items: Vec<Result<T, CompilerError>>) -> Option<Vec<T>> {
    let mut successful_results = Vec::new();
    let mut has_error = false;

    for result in items {
        match result {
            Ok(value) => successful_results.push(value),
            Err(error) => {
                has_error = true;
                eprintln!("ERROR: {}", error);
            }
        }
    }

    if has_error {
        None
    } else {
        Some(successful_results)
    }
}

fn main() {
    let args = Args::parse();
    match args.file_path {
        Some(path) => {
            if args.tokens {
                let mut lexer = Lexer::new();
                let path = Path::new(&path);
                let mut source_file = SourceFile::new(path);
                lexer.set_content(&mut source_file);
                let tokens: Vec<Result<Token, CompilerError>> = lexer.collect();
                println!("format: [line:start-end] token");
                println!("{:#?}", tokens);
            }
            if args.compile {
                let mut parser = ngin::parser::Parser::new();
                let path = Path::new(&path);
                let mut source_file = SourceFile::new(path);
                parser.set_content(&mut source_file);
                let ast_attempt: Vec<Result<Node, CompilerError>> = parser.collect();
                let maybe_ast = handle_error(ast_attempt);
                if let Some(ast) = maybe_ast {
                    let validate_result = validate(ast);
                    match validate_result {
                        Ok(compile_ready_ast) => {
                            println!("{:#?}", compile_ready_ast);
                        }
                        Err(compiler_error) => {
                            eprint!("{}", compiler_error)
                        }
                    }
                }
            }

            if args.debug {
                let mut parser = ngin::parser::Parser::new();
                let path = Path::new(&path);
                let mut source_file = SourceFile::new(path);
                parser.set_content(&mut source_file);
                let ast: Vec<Result<Node, CompilerError>> = parser.collect();

                println!("{:#?}", ast);
            }

            if !args.debug && !args.tokens && !args.compile {
                let mut runtime = Ngin::new();
                let body = runtime.include(&path);
                runtime.execute(&body);
            }
        }
        None => println!("starting repl"),
    }
}
