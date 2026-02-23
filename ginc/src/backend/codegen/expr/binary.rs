//! Lower binary operation expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for Binary {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let lhs = self.lhs.lower(ctx, block, symtab)?;
        let rhs = self.rhs.lower(ctx, block, symtab)?;

        Ok(match self.op {
            BinOp::Add => block.append_op(ctx.mlir.build_binop(ArithOps::ADD, lhs, rhs)),
            BinOp::Subtract => block.append_op(ctx.mlir.build_binop(ArithOps::SUB, lhs, rhs)),
            BinOp::Multiply => block.append_op(ctx.mlir.build_binop(ArithOps::MUL, lhs, rhs)),
            BinOp::Divide => block.append_op(ctx.mlir.build_binop(ArithOps::DIV, lhs, rhs)),
            BinOp::Equal => block.append_op(ctx.mlir.build_cmpi(Predicates::EQ, lhs, rhs)),
            BinOp::NotEqual => block.append_op(ctx.mlir.build_cmpi(Predicates::NE, lhs, rhs)),
            BinOp::LessThan => block.append_op(ctx.mlir.build_cmpi(Predicates::SLT, lhs, rhs)),
            BinOp::GreaterThan => block.append_op(ctx.mlir.build_cmpi(Predicates::SGT, lhs, rhs)),
            BinOp::LessThanOrEqual => {
                block.append_op(ctx.mlir.build_cmpi(Predicates::SLE, lhs, rhs))
            }
            BinOp::GreaterThanOrEqual => {
                block.append_op(ctx.mlir.build_cmpi(Predicates::SGE, lhs, rhs))
            }
        })
    }
}
