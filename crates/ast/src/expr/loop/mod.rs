pub mod r#for;
pub use r#for::*;

pub mod r#while;
pub use r#while::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Loop {
    While(WhileLoop),
    ForIn(ForInLoop),
}
