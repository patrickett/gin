use std::collections::HashMap;

use super::Expr;

#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    Bool(bool),
    // list of properties on object to destructure {x,y,z} -> [x,y,z]
    DestructureObject(Vec<String>),
    List(Vec<Expr>),
    Number(usize),
    Object(HashMap<String, Expr>),
    String(String),
    TemplateString(String),
}

impl std::fmt::Display for Literal {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match self {
            Literal::DestructureObject(_) => todo!(),
            Literal::Object(_) => todo!(),
            Literal::List(_) => todo!(),
            Literal::TemplateString(_) => todo!(),
            Literal::Bool(b) => write!(fmt, "{}", b),
            Literal::String(s) => write!(fmt, "{}", s),
            Literal::Number(n) => write!(fmt, "{}", n),
        }
    }
}
