// use std::fs::canonicalize;
// use std::path::Path;
use std::{borrow::BorrowMut, collections::HashMap, path::Path};
pub mod gin_type;
pub mod parser;
pub mod source_file;
pub mod user_input;
mod value;

use self::{
    parser::{
        module::{
            definition::Define,
            expression::{Binary, Expr, Op},
            Node,
        },
        Parser,
    },
    source_file::SourceFile,
    value::GinValue,
};

// TODO: files needs to be able to check last_modified
// if the file is open in another buffer (has write or read lock)
//
// behind the scenes we should transition to machine compiled code
// JIT compile where possible but jit code that can interface with the runtime
// provide errors and state so that we can recover in the runtime
//
// basically we have a development runtime that can be optionally removed
// for a build

pub struct Ngin {
    files: HashMap<String, SourceFile>,
    parser: Parser,
    scope: HashMap<String, Vec<Node>>,
}

// no compile run cycle. compile inside of the program
// NO blank slate run to termination
// all program state is saved and can be revived on reboot
// this means you can change and debug things while its running
//
// runtime introspection
// catch errors as they happen give option to fix and continue

// TODO: ask if its okay to read a directory or file

impl Ngin {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            scope: HashMap::new(),
            files: HashMap::new(),
        }
    }

    /// Prints to the console for the user to see
    pub fn print_error(&self, error: String) {
        eprintln!("{}", error)
    }

    pub fn include(&mut self, path: &String) -> Vec<Node> {
        if let Some(file) = self.files.get(path) {
            let module = file.to_module(&mut self.parser);
            return module.body;
        }

        let path = Path::new(&path);

        if !path.exists() {
            // TODO: prompt user don't error
            self.print_error(format!("No such file or directory: {}", path.display()));
            std::process::exit(1)
        }

        let source_file = SourceFile::new(path);

        self.parser.set_content(&source_file);
        let full_path = source_file.full_path().to_string();
        self.files.insert(full_path, source_file);
        let ast = self.parser.borrow_mut().collect();

        ast
    }

    /// compile a function to llvm ir (JIT?)
    // pub fn compile_function() {}

    pub fn execute(&mut self, body: &Vec<Node>) -> GinValue {
        let mut res = GinValue::Nothing;

        for node in body {
            match node {
                Node::Expression(expr) => {
                    res = self.evaluate(&expr);
                }
                Node::Definition(def) => match def {
                    Define::Data(_data) => todo!(),
                    Define::Function(func) => {
                        self.scope
                            .insert(func.name.to_owned(), func.body.to_owned());
                    }
                },
                Node::Statement(_) => todo!(),
            }
        }
        res
    }

    pub fn call(&mut self, name: &String, _arg: Option<Box<Expr>>) -> GinValue {
        if let Some(body) = self.scope.clone().get(name) {
            self.execute(body)
        } else {
            panic!("Unknown function name: {}", name)
        }
    }

    pub fn evaluate(&mut self, expr: &Expr) -> GinValue {
        match expr {
            Expr::Call(call) => {
                if call.name.as_str() == "print" {
                    if let Some(arg) = &call.arg {
                        // println!("print {:#?}", &arg);
                        println!("{}", self.evaluate(arg));
                    }
                    GinValue::Nothing
                } else {
                    self.call(&call.name, call.arg.to_owned())
                }
            }
            Expr::Literal(lit) => match lit {
                GinValue::Bool(_) => todo!(),
                GinValue::String(_) => todo!(),
                GinValue::Number(_) => todo!(),
                GinValue::Nothing => todo!(),
                GinValue::TemplateString(_) => todo!(),
                GinValue::Object(_) => todo!(),
                // Literal::Data(_) => todo!(),
                // Literal::List(_) => todo!(),
                // Literal::TemplateString(_) => todo!(),
                // Literal::Bool(b) => GinValue::Bool(*b),
                // Literal::String(s) => GinValue::String(s.to_owned()),
                // Literal::Number(num) => GinValue::Number(*num),
                // Literal::DestructureData(_) => todo!(),
            },
            Expr::Operation(lhs, op, rhs) => match op {
                Op::Compare(_) => todo!(),
                Op::Bin(binop) => match binop {
                    Binary::Add => self.evaluate(&lhs) + self.evaluate(&rhs),
                    Binary::Sub => todo!(),
                    Binary::Div => todo!(),
                    Binary::Mul => todo!(),
                },
            },
        }
    }
}
