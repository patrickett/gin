use std::{env, fs, path::Path};

use crate::parser::Parser;
// mod compiler;
mod lex;
// mod mlir;
mod parser;

fn main() {
    if let Some(path) = env::args().nth(1) {
        let path = Path::new(&path);
        let src = fs::read_to_string(path).expect("Invalid path");

        let file_name = path
            .file_stem()
            .expect("Failed to get filename")
            .to_str()
            .expect("Failed to convert filename to str");

        // println!("{}", file_name);
        // let tokens1: Vec<Token> = lex::Tokenizer::new(src.clone()).collect();
        let mut lexer = lex::Lexer::new();
        let tokens = lexer.lex(&src);
        // println!("{:#?}", &tokens);
        // println!("{:#?}", &tokens1);
        // a file exports a default empty module with the name of the file
        
        let mut parser = Parser::new();

        let ast = parser.parse(&tokens);
        // let exprs = parser::parse_mod(tokens, file_name.to_string());
        println!("ast: \n{:#?}", ast);

        // let ir = compiler::compile_exprs(exprs, file_name);
        // let res = evaluate_exprs(exprs);
    } else {
        eprintln!("Failed to provide gin file")
    }
}
