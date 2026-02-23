use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: ModPath,
    pub args: Option<Vec<Expr>>,
}

pub fn fn_call<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, FnCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let args = delimited_list(ParenOpen, expr, Comma, ParenClose).or_not();

    path()
        .then(args)
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| FnCall { path, args })
}
