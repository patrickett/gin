use super::control_flow::ControlFlow;

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
