use std::{borrow::BorrowMut, collections::HashMap, path::Path};
pub mod compiler_error;
pub mod gin_type;
pub mod parser;
pub mod source_file;
pub mod user_input;
pub mod validator;
mod value;

use self::{
    parser::{
        ast::{definition::Define, expression::Expr, Node},
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
        let temp_path = path.clone();
        if let Some(&mut ref mut file) = self.files.get_mut(&temp_path) {
            return file.to_module(&mut self.parser);
        }

        let path = Path::new(&path);

        if !path.exists() {
            // TODO: prompt user don't error
            self.print_error(format!("No such file or directory: {}", path.display()));
            std::process::exit(1)
        }

        let mut source_file = SourceFile::new(path);

        self.parser.set_content(&mut source_file);
        let full_path = source_file.full_path().to_string();
        self.files.insert(full_path, source_file);
        let ast: Vec<Result<Node, compiler_error::CompilerError>> =
            self.parser.borrow_mut().collect();

        vec![]
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
                    Define::Record { .. } => todo!(),
                    Define::Function {
                        name,
                        body,
                        returns: _,
                    } => {
                        self.scope.insert(name.to_owned(), body.to_owned());
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
            Expr::Call { name, arg } => {
                if name.as_str() == "print" {
                    if let Some(arg) = &arg {
                        // println!("print {:#?}", &arg);
                        println!("{}", self.evaluate(arg));
                    }
                    GinValue::Nothing
                } else {
                    self.call(&name, arg.to_owned())
                }
            }
            Expr::Literal(lit) => lit.clone(),
            Expr::Arithmetic(_) => todo!(),
            Expr::Relational(_) => todo!(),
            Expr::If {
                cond,
                true_body,
                false_body,
            } => {
                let cond_result = self.evaluate(cond);
                match cond_result {
                    GinValue::Bool(b) => {
                        if b {
                            self.execute(true_body)
                        } else {
                            if let Some(false_body) = false_body {
                                self.execute(false_body)
                            } else {
                                GinValue::Nothing
                            }
                        }
                    }
                    _ => todo!(),
                }
            }
        }
    }
}
