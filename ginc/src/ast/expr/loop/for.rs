use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::{parse::block, prelude::*};
use chumsky::span::SimpleSpan;

/// For-in loop: iterate over a range or collection
///
/// Example:
/// ```gin
/// main:
///     for item in items
///     loop
/// return
/// ```
/// OR like a range
/// ```gin
/// main:
///     for i in 1...50
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForInLoop {
    pub pat: Pattern,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Expr>,
    pub exprs: Vec<Expr>,
}

pub fn for_loop_header_expr<'t, I>() -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;
    use chumsky::pratt::{infix, left};

    let atom = recursive(|expr| {
        choice((
            literal().map(Expr::Lit).boxed(),
            fn_call(expr.clone()).map(Expr::FnCall).boxed(),
        ))
    });

    // Range operator (precedence 2)
    let range = infix(left(2), just(Infer), |lhs: Expr, _, rhs: Expr, _| {
        Expr::Range(Range::new(lhs, rhs))
    });

    // Arithmetic operators (precedence 3)
    let arithmetic = infix(
        left(3),
        select! {
            Plus => BinOp::Add,
            Minus => BinOp::Subtract,
            Star => BinOp::Multiply,
            Slash => BinOp::Divide,
            Percent => BinOp::Modulo,
        },
        |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
    );

    atom.pratt((range, arithmetic))
        .padded_by(just(Newline).repeated())
}

impl<'c> Lower<'c> for ForInLoop {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let loc = ctx.location();
        let index_ty = Type::index(ctx.mlir);

        // Currently only range iterators (start...end) are supported.
        let (start_expr, end_expr) = match self.iter.as_ref() {
            Expr::Range(range) => (&range.start, &range.end),
            _ => {
                ctx.emit_symptom(CodegenSymptom::Internal {
                    message: "for-in loops currently only support range iterators (start...end)"
                        .to_string(),
                    span: SimpleSpan::new((), 0..0),
                });
                return None;
            }
        };

        // Lower bounds to i64, then cast to index for scf.for.
        let start_i64 = start_expr.lower(ctx, block, symtab)?;
        let end_i64 = end_expr.lower(ctx, block, symtab)?;
        let step_i64 = block.const_i64(ctx.mlir, 1);

        let start_idx = block.append_op(arith_dialect::index_cast(start_i64, index_ty, loc));
        let end_idx = block.append_op(arith_dialect::index_cast(end_i64, index_ty, loc));
        let step_idx = block.append_op(arith_dialect::index_cast(step_i64, index_ty, loc));

        // Build the loop body region.
        // scf.for provides an index-typed induction variable as the block argument.
        let loop_region = Region::new();
        {
            let loop_blk = Block::new(&[(index_ty, loc)]);
            loop_region.append_block(loop_blk);
            let loop_blk_ref = loop_region.first_block().unwrap();

            // Cast induction variable from index to i64 and bind to the loop pattern.
            let iv: Value = loop_blk_ref.argument(0).unwrap().into();
            let iv_i64 = loop_blk_ref.append_op(arith_dialect::index_cast(iv, ctx.mlir.i64(), loc));

            let mut loop_symtab = symtab.clone();
            match &self.pat {
                Pattern::Ident(name) => {
                    loop_symtab.insert(name.as_str().to_string(), iv_i64);
                }
                Pattern::Tuple(_) => {
                    ctx.emit_symptom(CodegenSymptom::Internal {
                        message: "Tuple patterns in for loops are not yet supported".to_string(),
                        span: SimpleSpan::new((), 0..0),
                    });
                    return None;
                }
            }

            for expr in &self.exprs {
                expr.lower(ctx, &loop_blk_ref, &mut loop_symtab)?;
            }

            loop_blk_ref.append_operation(scf_dialect::r#yield(&[], loc));
        }

        block.append_operation(scf_dialect::r#for(
            start_idx,
            end_idx,
            step_idx,
            loop_region,
            loc,
        ));

        Some(block.const_i64(ctx.mlir, 0))
    }
}

pub fn for_in_loop<'t, I>(
    header_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    body_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, ForInLoop, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let header = just(For)
        .ignore_then(pattern())
        .then_ignore(just(In))
        .then(header_expr.clone().map(Box::new));
    let body = body_expr.clone();
    let end = just(Token::Loop);

    block(header, body, end).map(|((pat, iter), exprs, _loop)| ForInLoop { pat, iter, exprs })
}
