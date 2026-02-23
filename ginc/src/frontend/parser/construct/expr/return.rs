use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*, span::SimpleSpan, Parser};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return(pub Option<Box<Expr>>);

pub fn r#return<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Return, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    // try bare return first so it doesn't consume trailing newlines
    let bare = just(Token::Return)
        .then_ignore(just(Token::Newline))
        .to(Return(None));

    let with_value = just(Token::Return)
        .ignore_then(expr)
        .map(|e| Return(Some(Box::new(e))));

    choice((bare, with_value))
}
