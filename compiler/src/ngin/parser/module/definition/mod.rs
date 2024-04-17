use std::collections::HashMap;

use crate::ngin::gin_type::{GinType, GinTyped};

use super::Node;

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Node>,
    /// Return type can optionally be set by the user.
    /// otherwise it can be contextually infered if lets say there is only a single
    /// expression the function body. Beyond that needs to be infered from the body
    pub return_type: GinType,
}

impl Function {
    pub fn new_with_return_type(name: String, body: Vec<Node>, return_type: GinType) -> Self {
        // TODO: infer body still, check if it matches with the return_type
        Self {
            name,
            body,
            return_type,
        }
    }

    pub fn new(name: String, body: Vec<Node>) -> Self {
        // TODO: infer return_type from body

        let mut return_type = GinType::Nothing;
        if body.len() == 1 {
            if let Some(expr) = body.get(0) {
                return_type = expr.gin_type(None);
            }
        } else {
            // TODO: multi line fn body (needs to handle control flow)
        }

        Self {
            name,
            body,
            return_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// This is type information. not runtime value
pub struct DataDefiniton {
    pub name: String,
    pub body: HashMap<String, GinType>,
}

impl DataDefiniton {
    pub fn new(name: String) -> Self {
        Self {
            name,
            body: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, gin_type: GinType) -> Option<GinType> {
        self.body.insert(name, gin_type)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    Data(DataDefiniton),
    Function(Function),
}
