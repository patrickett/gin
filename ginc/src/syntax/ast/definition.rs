use super::Node;
use crate::gin_type::GinType;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Union {}

#[derive(Debug, Clone, PartialEq)]
pub struct Range {}

#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    Record(Record),
    Function(Function),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    name: String,
    kind: GinType,
}

impl Parameter {
    pub fn new(name: String, kind: GinType) -> Self {
        Self { name, kind }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    name: String,
    parameter: Option<Parameter>,
    explicit_return: Option<GinType>,
    pub body: Vec<Node>,
}

impl Function {
    pub fn new(
        name: String,
        parameter: Option<Parameter>,
        explicit_return: Option<GinType>,
    ) -> Self {
        Self {
            name,
            parameter,
            body: Vec::new(),
            explicit_return,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    name: String,
    pub body: HashMap<String, GinType>,
}

impl Record {
    pub fn new(name: String) -> Self {
        Self {
            body: HashMap::new(),
            name,
        }
    }
}
