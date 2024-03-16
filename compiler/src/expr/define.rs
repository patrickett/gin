use std::collections::HashMap;

use crate::gin_type::GinType;

use super::Expr;

#[derive(Debug, Clone, PartialEq)]
pub enum Define {
    /// Object Name, Object Type Defintions
    Data(String, HashMap<String, GinType>),

    DataContent(HashMap<String, GinType>),

    /// Name, Body, ReturnType
    Function(String, Vec<Expr>, GinType),
}
