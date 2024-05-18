use self::{definition::Define, expression::Expr, statement::Statement};
use crate::gin_type::GinTyped;
use crate::parser::GinType;
pub mod control_flow;
pub mod definition;
pub mod expression;
pub mod statement;

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Expression(Expr),
    Definition(Define),
    Statement(Statement),
}

impl GinTyped for Node {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType {
        match self {
            Node::Expression(e) => e.gin_type(context),
            Node::Definition(_) => GinType::Nothing,
            Node::Statement(_) => GinType::Nothing,
        }
    }
}

impl GinTyped for Vec<Node> {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType {
        match context {
            Some(context) => {
                if context.len() == 1 {
                    if let Some(Node::Expression(expr)) = context.last() {
                        expr.gin_type(Some(&context))
                    } else {
                        GinType::Nothing
                    }
                } else {
                    if let Some(Node::Expression(expr)) = context.last() {
                        expr.gin_type(Some(&context))
                    } else {
                        GinType::Nothing
                    }
                }
            }
            None => todo!(),
        }
    }
}
