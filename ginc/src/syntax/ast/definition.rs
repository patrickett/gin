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

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    name: String,
    parameter: Option<Parameter>,
    pub body: Vec<Node>,
    pub returns: GinType,
}

impl Function {
    pub fn new(name: String, parameter: Option<Parameter>, returns: GinType) -> Self {
        Self {
            name,
            parameter,
            body: Vec::new(),
            returns,
        }
    }

    // TODO: check body to find the actual return type
    // CompilerError when mismtach
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    name: String,
    body: HashMap<String, GinType>,
}

impl Record {
    pub fn new(name: String) -> Self {
        Self {
            body: HashMap::new(),
            name,
        }
    }

    pub fn insert(&mut self, k: String, v: GinType) -> Option<GinType> {
        self.body.insert(k, v)
    }
}
