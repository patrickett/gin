use crate::ngin::{
    gin_type::{GinType, GinTyped},
    value::GinValue,
};

use super::Node;

#[derive(Debug, Clone, PartialEq)]
/// Expressions are things that can be evaluated to a value.
pub enum Expr {
    If {
        /// boolean condition
        cond: Box<Expr>,
        true_body: Vec<Node>,
        false_body: Option<Vec<Node>>,
    },
    Call {
        name: String,
        arg: Option<Box<Expr>>,
    },
    Arithmetic(Box<ArithmeticExpr>),
    Relational(Box<RelationalExpr>),
    Literal(GinValue),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RelationalExpr {
    LessThan { lhs: Expr, rhs: Expr },
    LessThanOrEqualTo { lhs: Expr, rhs: Expr },
    GreaterThan { lhs: Expr, rhs: Expr },
    GreaterThanOrEqualTo { lhs: Expr, rhs: Expr },
    Equals { lhs: Expr, rhs: Expr },
    NotEquals { lhs: Expr, rhs: Expr },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArithmeticExpr {
    Add { lhs: Expr, rhs: Expr },
    Sub { lhs: Expr, rhs: Expr },
    Div { lhs: Expr, rhs: Expr },
    Mul { lhs: Expr, rhs: Expr },
}

impl GinTyped for Expr {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType {
        match self {
            Expr::Call {name, arg} => match context {
                Some(ctx) => ctx.gin_type(context),
                None => panic!("function call: '{:#?}' must be provided with context to determine what its calling and returning", self),
            }
            Expr::Literal(lit) => match lit {
                GinValue::TemplateString(_) => todo!(),
                GinValue::Object(_) => todo!(),
                GinValue::Bool(_) => GinType::Bool,
                GinValue::String(_) => GinType::String,
                GinValue::Number(_) => GinType::Number,
                GinValue::Nothing => todo!(),
            },
            Expr::Arithmetic(_) => GinType::Number,
            Expr::Relational(_) => GinType::Bool,
            Expr::If { cond: _, true_body, false_body } => {
                let false_return = if false_body.is_none() {
                    GinType::Nothing
                } else {
                    GinType::Nothing
                };

                let true_return = true_body.gin_type(None);

                GinType::create_union(vec![true_return, false_return])
            },
        }
    }
}
