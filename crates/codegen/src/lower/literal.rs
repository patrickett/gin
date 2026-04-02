use crate::prelude::*;

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
                let mlir_ty = if *n > i64::MAX as u128 {
                    ctx.mlir.i128()
                } else {
                    ctx.mlir.i64()
                };
                block.const_int(ctx.mlir, mlir_ty, *n as i128)
            }
            Literal::Number(n) => block.const_i64(ctx.mlir, *n as i64),
            Literal::Float(f) => {
                block.append_op(ctx.mlir.const_op(ctx.mlir.f64_attr(*f), ctx.mlir.f64()))
            }
            Literal::String(s) => block.const_string_with_ctx(ctx, s),
        })
    }
}
