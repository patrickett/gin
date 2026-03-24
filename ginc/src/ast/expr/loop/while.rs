use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::{parse::block, prelude::*};

/// While loop: loop while a condition holds.
///
/// ```gin
/// main:
///     while x < 10
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WhileLoop {
    pub cond: Box<Expr>,
    pub exprs: Vec<Expr>,
}

pub fn while_loop<'t, I>(
    body_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, WhileLoop, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    block(
        just(Token::While).ignore_then(body_expr.clone()),
        body_expr,
        just(Token::Loop),
    )
    .map(|(cond, exprs, _)| WhileLoop { cond: Box::new(cond), exprs })
}

impl<'c> Lower<'c> for WhileLoop {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        let loc = ctx.location();

        // scf.while with no loop-carried values: () -> ()
        //
        // before-region: evaluate condition, call scf.condition
        // after-region:  execute body, call scf.yield
        let before_region = Region::new();
        {
            let blk = Block::new(&[]);
            before_region.append_block(blk);
            let blk_ref = before_region.first_block().unwrap();
            let cond_val = self.cond.lower(ctx, &blk_ref, &mut symtab.clone())?;
            blk_ref.append_operation(scf_dialect::condition(cond_val, &[], loc));
        }

        let after_region = Region::new();
        {
            let blk = Block::new(&[]);
            after_region.append_block(blk);
            let blk_ref = after_region.first_block().unwrap();
            let mut body_symtab = symtab.clone();
            for expr in &self.exprs {
                expr.lower(ctx, &blk_ref, &mut body_symtab)?;
            }
            blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
        }

        block.append_operation(scf_dialect::r#while(&[], &[], before_region, after_region, loc));

        Ok(block.const_i64(ctx.mlir, 0))
    }
}
