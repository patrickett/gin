use crate::frontend::prelude::*;

#[derive(Debug, Clone, Default)]
/// Output of parsing a gin file
pub struct GinAST(pub Vec<Item>);
