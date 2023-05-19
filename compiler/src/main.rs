use std::{env, fs};
mod lex;
mod parser;

fn run(path: String) {
    let exprs = compile(path);
}

fn compile(path: String) {
    let src_code = fs::read_to_string(path).expect("Invalid path");
    let tokens = lex::tokenize(src_code);
    let exprs = parser::parse(tokens);
    println!("{:#?}", exprs);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() == 1 {
        eprintln!("Failed to provide any arguements")
    } else {
        if let (Some(cmd), Some(path)) = (args.get(1), args.get(2)) {
            let cmd = cmd.as_str();
            match cmd {
                "run" => run(path.to_owned()),
                "compile" => compile(path.to_owned()),
                u => println!("unknown command: {}", u),
            }
        } else {
            eprintln!("Not enough arguments...")
        }
    }
}
