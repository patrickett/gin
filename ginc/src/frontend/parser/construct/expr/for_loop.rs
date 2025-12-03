use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct ForInLoop {
    pub pat: Pattern,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Expr>,
    pub exprs: Vec<Expr>,
}

pub fn for_in_loop<'tokens, 'src: 'tokens, I>(
    expr: impl Parser<'tokens, I, Expr, ParserError<'tokens, 'src>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, ForInLoop, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    use Token::*;

    just(For)
        .ignore_then(pattern())
        .then_ignore(just(In))
        .then(expr.clone().map(Box::new))
        .then_ignore(just(Newline))
        .then_ignore(just(Indent))
        .then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Dedent))
        .then_ignore(just(Newline).repeated().at_least(1).or_not())
        .then_ignore(just(Loop))
        .map(|((pat, iter), exprs)| ForInLoop { exprs, iter, pat })
}
