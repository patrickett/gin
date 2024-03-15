#![allow(unused)]
use clap::*;

use std::{env, fs, path::Path, process::exit};

use crate::{exit_status::ExitStatus, ngin::Ngin};

mod compiler;
mod exit_status;
mod expr;
mod gin_type;
mod lex;
mod module;
mod ngin;
mod parse;
mod tests;
pub mod token;

#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Path to the .gin file
    file_path: Option<String>,
    #[arg(short, long)]
    debug: bool,
}

fn main() {
    let args = Args::parse();
    match args.file_path {
        Some(p) => {
            let mut runtime = Ngin::new();
            let root_module = runtime.include(&p);
            if let Some(module) = root_module {
                if args.debug {
                    println!("{:#?}", module.get_body());
                } else {
                    let a = runtime.execute(&module.get_body());
                }
            } else {
                // TODO: handle better
                println!("unable to find module at location: {}", p);
            }
        }
        None => {
            println!("starting repl")
        }
    }
}
