use std::{collections::HashMap, str::FromStr};

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

impl FromStr for GinType {
    type Err = ();

    fn from_str(input: &str) -> Result<GinType, Self::Err> {
        match input {
            "number" => Ok(GinType::Number),
            "string" => Ok(GinType::String),
            "bool" => Ok(GinType::Bool),
            custom => Ok(GinType::Custom(custom.into())),
        }
    }
}

// impl std::fmt::Display for EvaluatedResult {
//     fn fmt(&self, fmt: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
//         match self {
//             EvaluatedResult::String(s) => write!(fmt, "{}", s),
//             EvaluatedResult::Number(n) => write!(fmt, "{}", n),
//             EvaluatedResult::Nothing => Ok(()),
//             EvaluatedResult::Bool(b) => write!(fmt, "{}", b),
//         }
//     }
// }
