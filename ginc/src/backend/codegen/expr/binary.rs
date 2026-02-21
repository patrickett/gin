//! Lower binary operation expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for Binary {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        if matches!(self.op, BinOp::Assign) {
            return lower_assign_expr(ctx, block, &self.lhs, &self.rhs, symtab);
        }

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
            BinOp::Range => {
                return Err(CodegenSymptom::Internal(
                    "Range expressions (..) should only appear in for loop iterators".to_string(),
                ));
            }
            BinOp::Assign => {
                return Err(CodegenSymptom::Internal(
                    "Assignment should be handled earlier".to_string(),
                ));
            }
        })
    }
}

fn lower_assign_expr<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    lhs: &Expr,
    rhs: &Expr,
    symtab: &mut RuntimeSymbolTable<'c>,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    let value = rhs.lower(ctx, block, symtab)?;

    if let Expr::FnCall(fn_call) = lhs {
        let var_name = if fn_call.path.segments.is_empty() {
            fn_call.path.root
        } else {
            let segs: Vec<&str> = fn_call.path.segments.iter().map(|s| s.as_str()).collect();
            IStr::new(format!("{}.{}", fn_call.path.root, segs.join(".")))
        };
        symtab.insert(var_name, value);
        Ok(value)
    } else {
        Err(CodegenSymptom::Internal(
            "Assignment LHS must be a variable name".to_string(),
        ))
    }
}
