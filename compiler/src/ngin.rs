use std::{collections::HashMap, path::Path, process::exit};

use crate::{
    exit_status::ExitStatus,
    expr::{Define, Expr, Literal},
    module::GinModule,
    parse::Parser,
};

pub struct Ngin {
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

pub enum GinValue {
    Bool(bool),
    String(String),
    Number(usize),
    Nothing,
}

impl std::fmt::Display for GinValue {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self {
            GinValue::Nothing => Ok(()),
            GinValue::String(s) => write!(fmt, "{}", s),
            GinValue::Number(n) => write!(fmt, "{}", n),
            GinValue::Bool(b) => write!(fmt, "{}", b),
        }
    }
}

impl Ngin {
    pub fn include(&mut self, path: &String) -> Option<GinModule> {
        let path = Path::new(&path);
        // TODO: prompt user don't error
        if !path.exists() {
            eprintln!("No such file or directory: {}", path.display());
            exit(ExitStatus::NoSuchFileOrDirectory.into());
        }
        Some(self.parser.start(path))
    }

    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            scope: HashMap::new(),
        }
    }

    /// compile a function to llvm ir (JIT?)
    pub fn compile_function() {}

    pub fn execute(&mut self, body: &Vec<Expr>) -> GinValue {
        let mut res = GinValue::Nothing;
        for expr in body {
            res = self.evaluate(&expr);
        }
        res
    }

    pub fn call(&mut self, name: &String) -> GinValue {
        if let Some(body) = self.scope.clone().get(name) {
            self.execute(body)
        } else {
            panic!("Unknown function name: {}", name)
        }
    }

    pub fn evaluate(&mut self, expr: &Expr) -> GinValue {
        match expr {
            Expr::Call(name, arg) => {
                if let Some(arg) = arg {
                    let v = self.evaluate(arg);
                    if name == "print" {
                        println!("{}", v);
                        GinValue::Nothing
                    } else {
                        todo!()
                    }
                } else {
                    self.call(name)
                }
            }
            Expr::Literal(lit) => match lit {
                Literal::Object(_) => todo!(),
                Literal::List(_) => todo!(),
                Literal::TemplateString(_) => todo!(),
                Literal::Bool(b) => GinValue::Bool(*b),
                Literal::String(s) => GinValue::String(s.to_owned()),
                Literal::Number(num) => GinValue::Number(*num),
                Literal::DestructureObject(_) => todo!(),
            },
            Expr::Define(def) => match def {
                Define::Function(name, body, _) => {
                    // push this to a hashmap, when called we evaluate the body
                    self.scope.insert(name.clone(), body.clone());
                    GinValue::Nothing
                }
                Define::Data(_, _) => todo!(),
                Define::DataContent(_) => todo!(),
            },
            Expr::Include(path, _) => todo!(),
            Expr::Opertation(left, op, right) => todo!(),
        }
    }
}
