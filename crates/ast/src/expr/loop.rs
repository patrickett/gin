use crate::expr::Typed;
use crate::expr::{Condition, Expr};
use crate::span::SubSpan;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForInLoop {
    /// Loop binder: a `for`-pattern-shaped [`Expr`] (see [`crate::pattern`]).
    /// Boxed to avoid a recursive `Expr` → `Loop` → `ForInLoop` → `Expr` layout.
    pub pat: Box<Typed<Expr>>,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Typed<Expr>>,
    pub exprs: Vec<Typed<Expr>>,
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhileLoop {
    pub condition: Condition,
    pub exprs: Vec<Typed<Expr>>,
    pub keyword_span: SubSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Loop {
    While(WhileLoop),
    ForIn(ForInLoop),
}
