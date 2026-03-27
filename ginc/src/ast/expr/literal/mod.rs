mod float;
mod int;
mod string;
mod unclosed_string;

pub use float::*;
pub use int::*;
pub use string::*;
pub use unclosed_string::*;

use crate::codegen::prelude::*;
use crate::prelude::*;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub enum Literal {
    Number(usize),
    Float(f64),
    Int(i64),
    String(String),
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl Hash for Literal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Float(f) => f.to_bits().hash(state),
            Self::Number(n) => n.hash(state),
            Self::Int(i) => i.hash(state),
            Self::String(s) => s.hash(state),
        }
    }
}

pub fn literal<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    choice((int(), float(), string(), unclosed_string()))
}

impl<'c> Lower<'c> for Literal {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        _symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        Some(match self {
            Literal::Int(n) => block.const_i64(ctx.mlir, *n),
            Literal::Number(n) => block.const_i64(ctx.mlir, *n as i64),
            Literal::Float(f) => {
                block.append_op(ctx.mlir.const_op(ctx.mlir.f64_attr(*f), ctx.mlir.f64()))
            }
            Literal::String(s) => block.const_string_with_ctx(ctx, s),
        })
    }
}
