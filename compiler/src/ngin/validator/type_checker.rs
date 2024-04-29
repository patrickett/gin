use crate::ngin::{
    compiler_error::CompilerError,
    gin_type::{GinType, GinTyped},
    parser::ast::{definition::Define, statement::Statement, Node},
};

pub struct TypeChecker;

impl TypeChecker {
    pub fn check_types(mut ast: Vec<Node>) -> Result<Vec<Node>, CompilerError> {
        for node in &mut ast {
            match node {
                Node::Expression(expr) => {
                    // println!("{:#?}", expr);
                }
                Node::Definition(def) => {
                    // println!("{:#?}", def);
                    match def {
                        Define::Record { .. } => todo!(),
                        Define::Function {
                            name: _,
                            body,
                            returns,
                        } => {
                            if *returns == GinType::Nothing {
                                // need to double check return type
                                let mut body_iter = body.iter();

                                let has_control_flow = body_iter.any(|n| {
                                    matches!(n, Node::Statement(Statement::ControlFlow(_)))
                                });

                                if has_control_flow {
                                    // traverse control flow
                                    // todo!();
                                } else {
                                    // implicit last expression return
                                    let last_node = body.last();
                                    if let Some(Node::Expression(expr)) = last_node {
                                        let rt = expr.gin_type(None);
                                        // println!("{:#?}", &rt);
                                        *returns = rt;
                                    } else {
                                        // println!("{:#?}", last_node)
                                    }
                                }
                            } else {
                                println!("not nothing")
                                // need to verify specified type is correct
                            }
                        }
                    }
                }
                Node::Statement(stmt) => {
                    // println!("{:#?}", stmt);
                }
            }
        }
        Ok(ast)
    }
}
