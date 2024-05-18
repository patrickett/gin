use self::verifier::Verifier;
use super::{compiler_error::CompilerError, syntax::ast::Node};
use crate::validator::type_checker::TypeChecker;

pub mod type_checker;
pub mod verifier;

pub fn validate(ast: Vec<Node>) -> Result<Vec<Node>, CompilerError> {
    let typed_ast = TypeChecker::check_types(ast)?;
    let valid_ast = Verifier::check_correctness(typed_ast)?;

    Ok(valid_ast)
}
