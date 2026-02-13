//! Lower literal expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for Literal {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        _symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        Ok(match self {
            Literal::Int(n) => block.const_i64(ctx.mlir, *n),
            Literal::Number(n) => block.const_i64(ctx.mlir, *n as i64),
            Literal::Float(f) => {
                block.append_op(ctx.mlir.const_op(ctx.mlir.f64_attr(*f), ctx.mlir.f64()))
            }
            Literal::String(s) => block.const_string_with_ctx(ctx, s),
            Literal::Ellipsis => block.const_i64(ctx.mlir, 0),
            Literal::Nothing => block.const_i64(ctx.mlir, 0),
        })
    }
}
