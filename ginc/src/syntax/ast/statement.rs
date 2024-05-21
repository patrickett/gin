use crate::{gin_type::GinType, syntax::ast::expression::Expr};

#[derive(Debug, Clone, PartialEq)]
pub enum ControlFlow {
    // boolean condition, body of condition, returntype
    WhenConditional(Expr, Vec<Expr>, GinType),
    // variable name
    WhenTypeIs(String),

    Return(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdOrDestructuredData {
    Id(String),
    DestructuredData(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Include(String, Option<IdOrDestructuredData>),
    ControlFlow(ControlFlow),
}
