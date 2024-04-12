// use std::fs::canonicalize;
// use std::path::Path;
use std::{borrow::BorrowMut, collections::HashMap};

mod value;
pub use crate::expr::define::Define;
pub use crate::{
    expr::{literal::Literal, Expr},
    module::GinModule,
    parse::Parser,
};
use crate::{
    expr::{Binary, Op},
    lexer::source_file::SourceFile,
};

use self::value::GinValue;

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
    scope: HashMap<String, Vec<Expr>>,
}

// no compile run cycle. compile inside of the program
// NO blank slate run to termination
// all program state is saved and can be revived on reboot
// this means you can change and debug things while its running
//
// runtime introspection
// catch errors as they happen give option to fix and continue

pub enum SourceFileError {
    FileNotFound,
}

impl Ngin {
    // TODO: read_file -> Result<SourceFile, UserDeny>

    /// This will create a stateful reader reference to a file
    /// on the filesystem.
    pub fn get_source_file(&mut self, path: String) -> SourceFile {
        SourceFile::new(path)
    }

    pub fn include(&mut self, path: String) -> GinModule {
        let source_file = self.get_source_file(path);
        self.parser.set_content(&source_file);
        let full_path = source_file.full_path().to_string();
        self.files.insert(full_path, source_file);
        let ast = self.parser.borrow_mut().collect();
        GinModule::new(ast)
    }

    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            scope: HashMap::new(),
            files: HashMap::new(),
        }
    }

    /// compile a function to llvm ir (JIT?)
    // pub fn compile_function() {}

    pub fn execute(&mut self, body: &Vec<Expr>) -> GinValue {
        let mut res = GinValue::Nothing;
        for expr in body {
            res = self.evaluate(&expr);
            // println!("res: {}", &res);
        }
        res
    }

    pub fn call(&mut self, name: &String, arg: Option<Box<Expr>>) -> GinValue {
        // println!("{:#?}", self.scope);
        // println!("{} {:#?}", name, arg);
        if let Some(body) = self.scope.clone().get(name) {
            self.execute(body)
        } else {
            panic!("Unknown function name: {}", name)
        }
    }

    pub fn evaluate(&mut self, expr: &Expr) -> GinValue {
        match expr {
            Expr::Call(name, arg) => {
                if name.as_str() == "print" {
                    if let Some(arg) = arg {
                        // println!("print {:#?}", &arg);
                        println!("{}", self.evaluate(arg));
                    }
                    GinValue::Nothing
                } else {
                    self.call(name, arg.to_owned())
                }
            }
            Expr::Literal(lit) => match lit {
                Literal::Data(_) => todo!(),
                Literal::List(_) => todo!(),
                Literal::TemplateString(_) => todo!(),
                Literal::Bool(b) => GinValue::Bool(*b),
                Literal::String(s) => GinValue::String(s.to_owned()),
                Literal::Number(num) => GinValue::Number(*num),
                Literal::DestructureData(_) => todo!(),
            },
            Expr::Define(def) => match def {
                Define::Function(name, body, _) => {
                    // push this to a hashmap, when called we evaluate the body
                    self.scope.insert(name.clone(), body.clone());
                    GinValue::Nothing
                }
                Define::Data(_, _) => {
                    // defining a structure really doesnt provide anything
                    // for the runtime in terms of values

                    GinValue::Nothing
                }
                Define::DataContent(_) => todo!(),
                Define::When() => todo!(),
            },
            Expr::Include(_, _) => todo!(),
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
