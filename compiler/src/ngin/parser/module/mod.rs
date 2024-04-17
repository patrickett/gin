use crate::ngin::gin_type::{GinType, GinTyped};

use self::{definition::Define, expression::Expr, statement::Statement};

pub mod definition;
pub mod expression;
pub mod statement;

/// The AST representation of the parsed file.
/// This is semantically the same as a GinModule
#[derive(Debug, Clone, PartialEq)]
pub struct GinModule {
    pub body: Vec<Node>,
}

impl GinModule {
    pub fn new(body: Vec<Node>) -> Self {
        Self { body }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Expression(Expr),
    Definition(Define),
    Statement(Statement),
}

impl GinTyped for Node {
    fn gin_type(&self, context: Option<&Vec<Node>>) -> GinType {
        match self {
            Node::Expression(e) => e.gin_type(context),
            Node::Definition(_) => GinType::Nothing,
            Node::Statement(_) => GinType::Nothing,
        }
    }
}

// fn find_last_expr_type(body: &Vec<Node>) -> GinType {
//     match body.last() {
//         Some(t) => match t {
//             // if we get a fncall we need find its decl
//             // then we return its return type
//             expr => expr.gin_type(Some(body)),
//         },
//         None => GinType::Nothing,
//     }
// }

impl GinTyped for Vec<Node> {
    fn gin_type(&self, _context: Option<&Vec<Node>>) -> GinType {
        todo!()
    }
}
