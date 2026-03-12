//! Lower for-in loop expressions to MLIR.

use crate::{backend::prelude::*, diagnostic::codegen::CodegenSymptom};

impl<'c> Lower<'c> for ForInLoop {
    fn lower(
        &self,
        _ctx: &CodegenContext<'_, 'c>,
        _block: &BlockRef<'c, 'c>,
        _symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        Err(CodegenSymptom::Internal(
            "For loop lowering not yet implemented".to_string(),
        ))
    }
}
