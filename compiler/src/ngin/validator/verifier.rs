use crate::ngin::{compiler_error::CompilerError, parser::ast::Node};

pub struct Verifier;

impl Verifier {
    pub fn check_correctness(typed_ast: Vec<Node>) -> Result<Vec<Node>, CompilerError> {
        Ok(typed_ast)
    }
}
