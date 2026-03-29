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
    Int(i128),
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
            // Emit integer constants at the width implied by the value.
            // Values fitting in i64 use i64; larger values use i128.
            // Note: melior's IntegerAttribute truncates to i64 internally
            Literal::Int(n) => {
                let mlir_ty = if *n > i64::MAX as i128 || *n < i64::MIN as i128 {
                    ctx.mlir.i128()
                } else {
                    ctx.mlir.i64()
                };
                block.const_int(ctx.mlir, mlir_ty, *n)
            }
            Literal::Number(n) => block.const_i64(ctx.mlir, *n as i64),
            Literal::Float(f) => {
                block.append_op(ctx.mlir.const_op(ctx.mlir.f64_attr(*f), ctx.mlir.f64()))
            }
            Literal::String(s) => block.const_string_with_ctx(ctx, s),
        })
    }
}
