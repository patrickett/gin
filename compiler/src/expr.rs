use std::collections::HashMap;

use crate::gin_type::GinType;

#[derive(Debug, Clone, PartialEq)]
pub enum IdOrDestructuredObject {
    Id(String),
    DestructuredObject(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Comparison {
    LessThan,
    LessThanOrEqualTo,
    GreaterThan,
    GreaterThanOrEqualTo,
    Equals,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Binary {
    Add,
    Sub,
    Div,
    Mul,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Compare(Comparison),
    Bin(Binary),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Include(String, Option<IdOrDestructuredObject>),
    /// FunctionName, Argument
    Call(String, Option<Box<Expr>>),
    Literal(Literal),
    Define(Define),
    Opertation(Box<Expr>, Op, Box<Expr>),
}

#[derive(Debug, PartialEq, Clone)]
pub enum Literal {
    // list of properties on object to destructure {x,y,z} -> [x,y,z]
    DestructureObject(Vec<String>),
    Object(HashMap<String, Expr>),
    List(Vec<Expr>),
    TemplateString(String),
    Bool(bool),
    String(String),
    Number(usize),
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

#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    /// Object Name, Object Type Defintions
    Data(String, HashMap<String, GinType>),

    DataContent(HashMap<String, GinType>),

    /// Name, Body, ReturnType
    Function(String, Vec<Expr>, GinType),
}

impl Expr {
    pub fn gin_type(&self) -> GinType {
        match self {
            Expr::Include(_, _) => todo!(),
            Expr::Call(function_name, _) => {
                // need to find the function and check its return type

                GinType::Nothing
            }
            Expr::Define(_) => GinType::Nothing,
            Expr::Literal(lit) => match lit {
                Literal::List(v) => {
                    let vals: Vec<GinType> = v.iter().map(|e| e.gin_type()).collect();

                    let mut deduplicated_vec = Vec::new();

                    for item in vals {
                        if !deduplicated_vec.contains(&item) {
                            deduplicated_vec.push(item);
                        }
                    }

                    GinType::List(deduplicated_vec)
                }
                Literal::Object(o) => {
                    let mut obj_def: HashMap<String, GinType> = HashMap::new();

                    for (key, expr) in o.iter() {
                        obj_def.insert(key.clone(), expr.gin_type());
                    }

                    GinType::Object(obj_def)
                }
                Literal::DestructureObject(_) => todo!(),
                Literal::String(_) => GinType::String,
                Literal::TemplateString(_) => GinType::String,
                Literal::Bool(_) => GinType::Bool,
                Literal::Number(_) => GinType::Number,
            },
            Expr::Opertation(left, op, right) => match op {
                Op::Compare(_) => GinType::Bool,
                Op::Bin(_) => {
                    let left = *left.to_owned();
                    let right = *right.to_owned();

                    if left.gin_type() != right.gin_type() {
                        // TODO: better warning
                        panic!("Operation is not operating on same type object")
                    }

                    // left=right
                    left.gin_type()
                }
            },
        }
    }
}
