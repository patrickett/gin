use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct Return(pub Option<Box<Expr>>);

pub fn r#return<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Return, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
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
