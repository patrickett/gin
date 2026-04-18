use crate::prelude::*;

impl<'c> Lower<'c> for ast::expr::AsmExpr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();

        let mut operand_values: Vec<Value<'c, 'c>> = Vec::with_capacity(self.operands.len());
        for operand in &self.operands {
            operand_values.push(operand.lower(ctx, block, symtab)?);
        }

        let bool_true = IntegerAttribute::new(IntegerType::new(ctx.mlir, 1).into(), 1).into();

        let asm_op = OperationBuilder::new("llvm.inline_asm", loc)
            .add_attributes(&[
                (
                    Identifier::new(ctx.mlir, "asm_string"),
                    StringAttribute::new(ctx.mlir, self.template.as_str()).into(),
                ),
                (
                    Identifier::new(ctx.mlir, "constraints"),
                    StringAttribute::new(ctx.mlir, self.constraints.as_str()).into(),
                ),
                (Identifier::new(ctx.mlir, "has_side_effects"), bool_true),
            ])
            .add_operands(&operand_values)
            .add_results(&[ctx.mlir.i64()])
            .build();

        match asm_op {
            Ok(op) => Some(block.append_op(op)),
            Err(e) => {
                ctx.emit_internal(format!("llvm.inline_asm: {e}"));
                None
            }
        }
    }
}
