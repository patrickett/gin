use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForInLoop {
    pub pat: Pattern,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Expr>,
    pub exprs: Vec<Expr>,
}

pub fn for_in_loop<'t, I>(
    header_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    body_expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, ForInLoop, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    block(
        // header
        just(For)
            .ignore_then(pattern())
            .then_ignore(just(In))
            .then(header_expr.clone().map(Box::new)),
        // body
        body_expr.clone(),
        // closer
        just(Token::Loop),
    )
    .map(|((pat, iter), exprs, _loop)| ForInLoop { pat, iter, exprs })
}
