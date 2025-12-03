use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct Comment(pub String);

/// A single comment can be a block of commented lines stacked on each other
pub fn comment<'t, 's: 't, I>() -> impl Parser<'t, I, Comment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    select! { Token::Comment(text) => text }
        .separated_by(just(Token::Newline))
        .collect::<Vec<_>>()
        .map(|c| Comment(c.join("\n")))
}

pub type Comments = Vec<Comment>;

// each comment is just a comment() + newline
pub fn comments<'t, 's: 't, I>() -> impl Parser<'t, I, Comments, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    comment()
        .separated_by(just(Token::Newline))
        .collect::<Vec<Comment>>()
}
