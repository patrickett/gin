use crate::ngin::{
    gin_type::{GinType, GinTyped},
    value::GinValue,
};

use super::Node;

#[derive(Debug, Clone, PartialEq)]
/// Call a function with arguments
pub struct Call {
    pub name: String,
    pub arg: Option<Box<Expr>>,
}

impl Call {
    pub fn new(name: String, arg: Option<Box<Expr>>) -> Self {
        Self { name, arg }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Expressions are things that can be evaluated to a value.
pub enum Expr {
    Call(Call),
    Operation(Box<Expr>, Op, Box<Expr>),
    Literal(GinValue),
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

impl GinTyped for Expr {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType {
        match self {
            Expr::Call(call) => match context {
                Some(ctx) => ctx.gin_type(context),
                None => panic!("function call: '{:#?}' must be provided with context to determine what its calling and returning", call),
            }
            Expr::Literal(lit) => match lit {
                GinValue::TemplateString(_) => todo!(),
                GinValue::Object(_) => todo!(),
                GinValue::Bool(_) => GinType::Bool,
                GinValue::String(_) => GinType::String,
                GinValue::Number(_) => GinType::Number,
                GinValue::Nothing => todo!(),
                // Literal::List(v) => {
                //     let vals: Vec<GinType> = v.iter().map(|e| e.gin_type(context)).collect();

                //     let mut deduplicated_vec = Vec::new();

                //     for item in vals {
                //         if !deduplicated_vec.contains(&item) {
                //             deduplicated_vec.push(item);
                //         }
                //     }

                //     GinType::List(deduplicated_vec)
                // }
                // Literal::Data(o) => {
                //     let mut obj_def: HashMap<String, GinType> = HashMap::new();

                //     for (key, expr) in o.iter() {
                //         obj_def.insert(key.clone(), expr.gin_type(context));
                //     }

                //     GinType::Object(obj_def)
                // }
                // Literal::DestructureData(_) => todo!(),
                // Literal::String(_) => GinType::String,
                // Literal::TemplateString(_) => GinType::String,
                // Literal::Bool(_) => GinType::Bool,
                // Literal::Number(_) => GinType::Number,
            },
            Expr::Operation(left, op, right) => match op {
                Op::Compare(_) => GinType::Bool,
                Op::Bin(_) => {
                    let left = *left.to_owned();
                    let right = *right.to_owned();

                    // type inference
                    // match &left {
                    //     Expr::Include(_, _) => panic!("cannot include with binop"),
                    //     Expr::Call(_, _) => todo!(),
                    //     Expr::Literal(l) => match l {
                    //         Literal::Bool(_) => todo!(),
                    //         Literal::DestructureData(_) => todo!(),
                    //         Literal::List(_) => todo!(),
                    //         Literal::Number(_) => match &right {
                    //             Expr::Include(_, _) => todo!(),
                    //             Expr::Call(_, _) => todo!(),
                    //             Expr::Literal(r) => match r {
                    //                 Literal::Bool(_) => todo!(),
                    //                 Literal::DestructureData(_) => todo!(),
                    //                 Literal::List(_) => todo!(),
                    //                 Literal::Number(_) => GinType::Number,
                    //                 Literal::Data(_) => todo!(),
                    //                 Literal::String(_) => todo!(),
                    //                 Literal::TemplateString(_) => todo!(),
                    //             },
                    //             Expr::Define(_) => todo!(),
                    //             Expr::Operation(_, _, _) => todo!(),
                    //         },
                    //         Literal::Data(_) => todo!(),
                    //         Literal::String(_) => todo!(),
                    //         Literal::TemplateString(_) => todo!(),
                    //     },
                    //     Expr::Define(_) => todo!(),
                    //     Expr::Operation(_, _, _) => todo!(),
                    // }

                    if left.gin_type(context) != right.gin_type(context) {
                        // TODO: better warning
                        panic!(
                            "Operation is not operating on same type object\nLHS: {:#?}\nRHS: {:#?}",
                            left.gin_type(context),
                            right.gin_type(context)
                        )
                    }

                    // left=right
                    left.gin_type(context)
                }
            },
        }
    }
}
