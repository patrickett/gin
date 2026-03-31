use crate::prelude::*;

impl<'c> Lower<'c> for Loop {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        match self {
            Loop::ForIn(for_loop) => for_loop.lower(ctx, block, symtab),
            Loop::While(while_loop) => while_loop.lower(ctx, block, symtab),
        }
    }
}
