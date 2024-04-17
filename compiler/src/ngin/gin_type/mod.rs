use std::{collections::HashMap, str::FromStr};

use super::parser::module::Node;
pub mod number;

#[derive(Debug, Clone, PartialEq)]
pub enum GinType {
    Bool,
    List(Vec<GinType>),
    Object(HashMap<String, GinType>),
    String,
    Number,
    // TODO: If a literal is unchanged in a function we should be able to return the actual value
    // we will refer to this as a constant since it is constant and unchanged
    // ConstantString(String),
    // ConstantNumber(usize),
    // ConstantObject(Map<String,>)
    Custom(String),
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GinTypeError {
    InvalidTypeName,
}

impl FromStr for GinType {
    type Err = GinTypeError;

    fn from_str(input: &str) -> Result<GinType, Self::Err> {
        match input {
            "Number" => Ok(GinType::Number),
            "String" => Ok(GinType::String),
            "Bool" => Ok(GinType::Bool),
            custom => {
                if let Some(c) = custom.chars().nth(0) {
                    if !c.is_uppercase() {
                        return Err(GinTypeError::InvalidTypeName);
                    }
                }

                Ok(GinType::Custom(custom.into()))
            }
        }
    }
}

pub trait GinTyped {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType;
}
