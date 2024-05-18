use std::{collections::HashMap, str::FromStr};

use crate::syntax::ast::Node;

pub mod number;

#[derive(Debug, Clone, PartialEq)]
pub enum GinType {
    /// Can be any ONE of the types within
    Union(Vec<GinType>),
    Bool,
    /// Can contain MULTIPLE of the types within
    List(Vec<GinType>),
    Record(HashMap<String, GinType>),
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

// impl Ord for GinType {
//     fn cmp(&self, other: &Self) -> std::cmp::Ordering {
//         match (self, other) {
//             (MyEnum::Variant1, MyEnum::Variant1) => std::cmp::Ordering::Equal,
//             (MyEnum::Variant1, _) => std::cmp::Ordering::Less,
//             (_, MyEnum::Variant1) => std::cmp::Ordering::Greater,
//             (MyEnum::Variant2(x), MyEnum::Variant2(y)) => x.cmp(y),
//             (MyEnum::Variant2(_), _) => std::cmp::Ordering::Less,
//             (_, MyEnum::Variant2(_)) => std::cmp::Ordering::Greater,
//             (MyEnum::Variant3(x), MyEnum::Variant3(y)) => x.cmp(y),
//         }
//     }
// }

impl GinType {
    /// Will only return a Union if there are 2 or more unique types
    /// after it is deduped. Otherwise it will return the single type.
    pub fn create_union(mut types: Vec<GinType>) -> GinType {
        if types.len() == 0 {
            GinType::Nothing
        } else if types.len() == 1 {
            types[0].clone()
        } else {
            // will dedup any repeating types
            // types.sort();
            types.dedup();

            GinType::Union(types)
        }
    }
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
