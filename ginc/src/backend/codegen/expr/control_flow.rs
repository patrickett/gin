//! Lower control flow expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for ControlFlow {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match self {
            ControlFlow::ForIn(for_loop) => lower_for_loop(ctx, block, for_loop, symtab),
            ControlFlow::If { .. } => Err(CodegenSymptom::Internal(
                "If/else lowering not yet implemented - TODO: Add scf.if support".to_string(),
            )),
            ControlFlow::While { .. } => Err(CodegenSymptom::Internal(
                "While loop lowering not yet implemented - TODO: Add scf.while support".to_string(),
            )),
        }
    }
}

fn lower_for_loop<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    for_loop: &ForInLoop,
    symtab: &mut RuntimeSymbolTable<'c>,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    let loop_var = match &for_loop.pat {
        Pattern::Ident(name) => name.clone(),
        Pattern::Tuple(_) => {
            return Err(CodegenSymptom::Internal(
                "Tuple patterns in for loops not yet implemented".to_string(),
            ));
        }
    };

    let (lower, upper) = match &*for_loop.iter {
        Expr::Binary(bin) if matches!(bin.op, BinOp::Range) => {
            let lb = bin.lhs.lower(ctx, block, symtab)?;
            let ub = bin.rhs.lower(ctx, block, symtab)?;
            (lb, ub)
        }
        _ => {
            return Err(CodegenSymptom::Internal(
                "For loop iterator must be a range expression (e.g., 1..100)".to_string(),
            ));
        }
    };

    let step = block.const_i64(ctx.mlir, 1);
    let loc = ctx.location();

    let region = Region::new();
    {
        let block = Block::new(&[(ctx.mlir.i64(), loc)]);
        region.append_block(block);
        let block_ref = region.first_block().unwrap();

        let iv = block_ref.argument(0).unwrap();
        let mut local_symtab = symtab.clone();
        local_symtab.insert(loop_var, iv.into());

        for expr in &for_loop.exprs {
            if let Err(e) = expr.lower(ctx, &block_ref, &mut local_symtab) {
                eprintln!("Error lowering loop body expression: {:?}", e);
            }
        }
    }

    let for_op = OperationBuilder::new("scf.for", loc)
        .add_operands(&[lower, upper, step])
        .add_regions([region])
        .build()
        .map_err(|e| CodegenSymptom::Internal(format!("Failed to build scf.for: {}", e)))?;

    block.append_operation(for_op);

    Ok(block.const_i64(ctx.mlir, 0)) // TODO: Support yielding values
}
