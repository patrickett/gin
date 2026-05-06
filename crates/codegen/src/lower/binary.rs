use crate::prelude::*;
use ::ast::{BinOp, Binary};
use typeck::TyInfer;

impl<'c> Lower<'c> for Binary {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let lhs = self.lhs.lower(ctx, block, symtab)?;
        let rhs = self.rhs.lower(ctx, block, symtab)?;

        let result_ty = lhs.r#type();
        let is_float = result_ty == ctx.mlir.f64();
        let unsigned = !is_float
            && self
                .lhs
                .as_ref()
                .infer_ty(&ctx.ty_env.infer_env(ctx))
                .is_unsigned_int();

        Some(match self.op {
            BinOp::Add => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::ADDF
                } else {
                    ArithOps::ADD
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Subtract => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::SUBF
                } else {
                    ArithOps::SUB
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Multiply => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::MULF
                } else {
                    ArithOps::MUL
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Divide => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::DIVF
                } else if unsigned {
                    ArithOps::DIVU
                } else {
                    ArithOps::DIV
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Modulo => block.append_op(ctx.mlir.build_binop(
                if is_float {
                    ArithOps::REMF
                } else if unsigned {
                    ArithOps::REMU
                } else {
                    ArithOps::REM
                },
                lhs,
                rhs,
                result_ty,
            )),
            BinOp::Equal => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OEQ, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::EQ, lhs, rhs)
            }),
            BinOp::NotEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::ONE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::NE, lhs, rhs)
            }),
            BinOp::LessThan => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OLT, lhs, rhs)
            } else if unsigned {
                ctx.mlir.build_cmpi(Predicates::ULT, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SLT, lhs, rhs)
            }),
            BinOp::GreaterThan => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OGT, lhs, rhs)
            } else if unsigned {
                ctx.mlir.build_cmpi(Predicates::UGT, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SGT, lhs, rhs)
            }),
            BinOp::LessThanOrEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OLE, lhs, rhs)
            } else if unsigned {
                ctx.mlir.build_cmpi(Predicates::ULE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SLE, lhs, rhs)
            }),
            BinOp::GreaterThanOrEqual => block.append_op(if is_float {
                ctx.mlir.build_cmpf(FPredicates::OGE, lhs, rhs)
            } else if unsigned {
                ctx.mlir.build_cmpi(Predicates::UGE, lhs, rhs)
            } else {
                ctx.mlir.build_cmpi(Predicates::SGE, lhs, rhs)
            }),
            BinOp::BitAnd => {
                block.append_op(ctx.mlir.build_binop(ArithOps::ANDI, lhs, rhs, result_ty))
            }
            BinOp::BitOr => {
                block.append_op(ctx.mlir.build_binop(ArithOps::ORI, lhs, rhs, result_ty))
            }
            BinOp::BitXor => {
                block.append_op(ctx.mlir.build_binop(ArithOps::XORI, lhs, rhs, result_ty))
            }
            BinOp::ShiftLeft => {
                block.append_op(ctx.mlir.build_binop(ArithOps::SHLI, lhs, rhs, result_ty))
            }
            BinOp::ShiftRight => block.append_op(ctx.mlir.build_binop(
                if unsigned {
                    ArithOps::SHRUI
                } else {
                    ArithOps::SHRI
                },
                lhs,
                rhs,
                result_ty,
            )),
        })
    }
}
