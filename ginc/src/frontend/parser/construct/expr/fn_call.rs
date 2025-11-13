use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct FnCall {
    pub path: Path,
    pub args: Option<Vec<Expr>>,
}

pub fn fn_call<'tokens, 'src: 'tokens, I>(
    expr: impl Parser<'tokens, I, Expr, ParserError<'tokens, 'src>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Expr, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    let args = expr
        .separated_by(just(Token::Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .or_not();

    path()
        .then(args)
        .then_ignore(just(Token::Newline).or_not())
        .map(|(path, args)| Expr::FnCall(FnCall { path, args }))
}
