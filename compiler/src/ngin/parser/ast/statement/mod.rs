use self::control_flow::ControlFlow;

pub mod control_flow;

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
