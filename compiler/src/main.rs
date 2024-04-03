use clap::*;
pub mod lexer;
pub mod ngin;
pub use crate::ngin::Ngin;

pub mod expr;
mod gin_type;

mod module;
mod parse;
mod tests;
pub mod token;

#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Path to the .gin file
    file_path: Option<String>,

    /// Print abstract syntax tree for the provided file
    #[arg(short, long)]
    debug: bool,
    //Print lexed tokens for the provided file
    // #[arg(short, long)]
    // tokens: bool,
}

fn main() {
    let args = Args::parse();
    match args.file_path {
        Some(path) => {
            let mut runtime = Ngin::new();
            let root_module = runtime.include(path);
            if !args.debug {
                runtime.execute(&root_module.get_body());
            } else {
                println!("{:#?}", root_module.get_body());
            }
        }
        None => println!("starting repl"),
    }
}
