use std::collections::HashMap;

mod value;
pub use crate::expr::define::Define;
use crate::lexer::source_file::SourceFile;
pub use crate::{
    exit_status::ExitStatus,
    expr::{literal::Literal, Expr},
    module::GinModule,
    parse::Parser,
};

use self::value::GinValue;

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

impl Ngin {
    pub fn include(&mut self, path: String) -> GinModule {
        let mut sf = SourceFile::new(path);
        let module = sf.to_module();
        self.files.insert(sf.full_path().to_owned(), sf);
        module
    }

    pub fn new() -> Self {
        Self {
            parser: Parser::new(None),
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

    pub fn println(&self, value: GinValue) -> GinValue {
        println!("{}", value);
        GinValue::Nothing
    }

    pub fn evaluate(&mut self, expr: &Expr) -> GinValue {
        match expr {
            Expr::Call(name, arg) => {
                if let Some(arg) = arg {
                    let v = self.evaluate(arg);
                    if name == "print" {
                        self.println(v)
                    } else {
                        println!("printing: ({}), name: {name}", v);
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
            Expr::Include(_, _) => todo!(),
            Expr::Opertation(lhs, op, rhs) => match op {
                crate::expr::Op::Compare(_) => todo!(),
                crate::expr::Op::Bin(binop) => match binop {
                    crate::expr::Binary::Add => self.evaluate(&lhs) + self.evaluate(&rhs),
                    crate::expr::Binary::Sub => todo!(),
                    crate::expr::Binary::Div => todo!(),
                    crate::expr::Binary::Mul => todo!(),
                },
            },
        }
    }
}
