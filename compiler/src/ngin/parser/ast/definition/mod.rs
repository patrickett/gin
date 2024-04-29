use std::collections::HashMap;

use crate::ngin::gin_type::GinType;

use super::Node;

#[derive(Debug, Clone, PartialEq)]
pub struct Union {}

#[derive(Debug, Clone, PartialEq)]
pub struct Range {}

#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    Record {
        name: String,
        body: HashMap<String, GinType>,
    },
    Function {
        name: String,
        body: Vec<Node>,
        returns: GinType,
    },
}
