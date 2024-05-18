mod compiler_error;
mod gin_type;
mod parser;
mod path_registry;
mod source_file;
mod user_input;
mod validator;
mod value;

use crate::parser::{lex::SimpleLexer, parse::SimpleParser};
use clap::*;
use compiler_error::CompilerError;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Path to the .gin file
    path: String,

    /// Print abstract syntax tree for the provided file
    #[arg(short, long)]
    ast: bool,
    /// Print lexed tokens for the provided file
    #[arg(short, long)]
    tokens: bool,
}

// pub fn extract_errors_and_nodes(
//     results: Vec<Result<Node, CompilerError>>,
// ) -> Result<Vec<Node>, Vec<CompilerError>> {
//     let (nodes, errors): (Vec<_>, Vec<_>) = results.into_iter().partition(Result::is_ok);
//     let nodes: Vec<Node> = nodes.into_iter().map(Result::unwrap).collect();
//     let errors: Vec<CompilerError> = errors.into_iter().map(Result::unwrap_err).collect();
//     if errors.is_empty() {
//         Ok(nodes)
//     } else {
//         Err(errors)
//     }
// }

fn main() -> Result<(), CompilerError> {
    let args = Args::parse();
    // let mut path_registry = path_registry::PathRegistry::new();

    let mut simple_lexer = SimpleLexer::new();
    let lexed_file = simple_lexer.lex(&args.path)?;

    if args.tokens {
        println!("{:#?}", lexed_file.tokens);
    }

    if args.ast {
        let mut parser = SimpleParser {};
        let parsed_file = parser.parse(&lexed_file)?;
        println!("{:#?}", parsed_file.nodes);
    }

    if !args.ast && !args.tokens {
        // let mut parser = parser::Parser::new();
        // let path = Path::new(&args.path);
        // let mut source_file = SourceFile::new(path);
        // parser.set_content(&mut source_file);
        // let ast_attempt: Vec<Result<Node, CompilerError>> = parser.collect();
        // let res = extract_errors_and_nodes(ast_attempt);
        // match res {
        //     Ok(ast) => print!("{:#?}", ast),
        //     Err(errors) => {
        //         for error in errors {
        //             eprintln!("error: {}", error);
        //         }
        //     }
        // }
    }
    Ok(())
}
