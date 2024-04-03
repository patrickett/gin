pub mod define;
pub mod literal;

use std::collections::HashMap;

use crate::gin_type::GinType;

use self::{define::Define, literal::Literal};

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Include(String, Option<IdOrDestructuredData>),
    /// FunctionName, Argument
    Call(String, Option<Box<Expr>>),
    Literal(Literal),
    Define(Define),
    Operation(Box<Expr>, Op, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdOrDestructuredData {
    Id(String),
    DestructuredData(Vec<String>),
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

impl Expr {
    pub fn gin_type(&self) -> GinType {
        match self {
            Expr::Include(_, _) => todo!(),
            Expr::Call(_, _) => {
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
                Literal::Data(o) => {
                    let mut obj_def: HashMap<String, GinType> = HashMap::new();

                    for (key, expr) in o.iter() {
                        obj_def.insert(key.clone(), expr.gin_type());
                    }

                    GinType::Object(obj_def)
                }
                Literal::DestructureData(_) => todo!(),
                Literal::String(_) => GinType::String,
                Literal::TemplateString(_) => GinType::String,
                Literal::Bool(_) => GinType::Bool,
                Literal::Number(_) => GinType::Number,
            },
            Expr::Operation(left, op, right) => match op {
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
