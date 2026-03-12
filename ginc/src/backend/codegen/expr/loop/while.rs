//! Lower while loop expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for WhileLoop {
    fn lower(
        &self,
        _ctx: &CodegenContext<'_, 'c>,
        _block: &BlockRef<'c, 'c>,
        _symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        Err(CodegenSymptom::Internal(
            "While loop lowering not yet implemented".to_string(),
        ))
    }
}
