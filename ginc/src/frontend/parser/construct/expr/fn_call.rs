use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: ModPath,
    pub args: Option<Vec<Expr>>,
}

pub fn fn_call<'tokens, I>(
    expr: impl Parser<'tokens, I, Expr, ParserError<'tokens>> + Clone + 'tokens,
) -> impl Parser<'tokens, I, Expr, ParserError<'tokens>>
where
    I: ValueInput<'tokens, Token = Token, Span = SimpleSpan>,
{
    use Token::*;

    let args = expr
        .separated_by(just(Comma))
        .collect::<Vec<_>>()
        .delimited_by(just(ParenOpen), just(ParenClose))
        .or_not();

    path()
        .then(args)
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| Expr::FnCall(FnCall { path, args }))
}
