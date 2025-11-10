use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct Comment {
    content: String,
}

pub fn comment<'t, 's: 't, I>() -> impl Parser<'t, I, Comment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    just(Token::DashDash)
        .then(
            any()
                .and_is(just(Token::Newline).not())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .map(|c| Comment {
            content: format!("{:#?}", c),
        })
}
