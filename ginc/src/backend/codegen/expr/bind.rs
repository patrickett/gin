//! Lower bind expressions (variable/function definitions) to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for Bind {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match &self.value() {
            BindValue::Body { exprs: _, ret: _ } => {
                let func_op = lower_function(ctx, &self.name(), self, None, false)?;
                block.append_operation(func_op);

                // Return a placeholder value (TODO: consider returning function reference)
                Ok(block.const_i64(ctx.mlir, 0))
            }
            BindValue::Expr(expr) => {
                let value = expr.lower(ctx, block, symtab)?;
                symtab.insert(self.name(), value);
                Ok(value)
            }
        }
    }
}
