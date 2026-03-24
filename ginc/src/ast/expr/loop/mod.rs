use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::prelude::*;

pub mod r#for;
pub use r#for::*;

pub mod r#while;
pub use r#while::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Loop {
    While(WhileLoop),
    ForIn(ForInLoop),
}

impl<'c> Lower<'c> for Loop {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match self {
            Loop::ForIn(for_loop) => for_loop.lower(ctx, block, symtab),
            Loop::While(while_loop) => while_loop.lower(ctx, block, symtab),
        }
    }
}

pub fn loop_expr<'t, I>(
    body_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Loop, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    choice((
        while_loop(body_expr.clone()).map(Loop::While),
        for_in_loop(for_loop_header_expr(), body_expr).map(Loop::ForIn),
    ))
    .boxed()
}
