use std::{env, fs};

use crate::ast::evaluate_exprs;

mod ast;
mod lex;
mod parser;

fn main() {
    if let Some(path) = env::args().nth(1) {
        let src_code = fs::read_to_string(path).expect("Invalid path");
        let tokens = lex::tokenize(src_code);
        // print!("{:#?}", &tokens);
        let exprs = parser::parse(tokens);
        let res = evaluate_exprs(exprs);
        println!("{:?}", res);
    } else {
        eprintln!("Failed to provide gin file")
    }
}
