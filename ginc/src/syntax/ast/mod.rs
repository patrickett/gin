pub mod definition;
pub mod expression;
pub mod statement;

use self::{definition::Define, expression::Expr, statement::Statement};

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Expression(Expr),
    Definition(Define),
    Statement(Statement),
}
