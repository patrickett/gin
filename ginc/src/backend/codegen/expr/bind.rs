//! Lower bind expressions (function definitions and tag bindings) to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for Bind {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        _symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match self {
            Bind::Def(name, params) => {
                // For function definitions, we generate a func.func operation
                // This is typically handled at the module level, not within expressions
                let func_op = lower_function(ctx, name, params)?;
                block.append_operation(func_op);

                // Return a placeholder value (TODO: consider returning function reference)
                Ok(block.const_i64(ctx.mlir, 0))
            }
            Bind::Tag(_, _) => {
                // TODO: Implement tag lowering
                // Tags are like decorators or metadata that can be applied to functions
                Err(CodegenSymptom::Internal(
                    "Tag lowering not yet implemented - TODO: Add decorator/tag support"
                        .to_string(),
                ))
            }
        }
    }
}
