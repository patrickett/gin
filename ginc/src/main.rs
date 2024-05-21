mod compiler_error;
mod gin_type;
mod syntax;
mod validator;
use clap::*;
use compiler_error::CompilerError;
use syntax::lex::SimpleLexer;

use crate::syntax::parse::SimpleParser;

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

fn main() -> Result<(), CompilerError> {
    let args = Args::parse();

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
        // todo: execute
    }
    Ok(())
}
