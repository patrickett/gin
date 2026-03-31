use crate::delimited_list;
use crate::prelude::*;
use chumsky::span::SimpleSpan;
use lexer::Token;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: ModPath,
    pub args: Option<Vec<Spanned<Expr>>>,
}

pub fn fn_call<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, FnCall, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let args = delimited_list(ParenOpen, expr, Comma, ParenClose).or_not();

    // Tag-rooted path (e.g. `Byte.new`, `Int.to_string`) takes priority so that
    // `Byte.new(...)` is not swallowed by the AnonymousTag arm first.
    let tag_fn = tag_path()
        .then(args.clone())
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| FnCall { path, args });

    let id_fn = path()
        .then(args)
        .then_ignore(just(Newline).or_not())
        .map(|(path, args)| FnCall { path, args });

    choice((tag_fn, id_fn))
}
