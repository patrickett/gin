use crate::{compiler_error::CompilerError, syntax::ast::Node};

pub struct Verifier;

impl Verifier {
    pub fn check_correctness(typed_ast: Vec<Node>) -> Result<Vec<Node>, CompilerError> {
        Ok(typed_ast)
    }
}
