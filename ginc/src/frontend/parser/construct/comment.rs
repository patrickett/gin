use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Comment(pub String);

/// A single comment can be a block of commented lines stacked on each other
pub fn comment<'t, I>() -> impl Parser<'t, I, Comment, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Comment(text) => text }
        .separated_by(just(Token::Newline))
        .collect::<Vec<_>>()
        .map(|c: Vec<&str>| Comment(c.join("\n")))
}

pub type Comments = Vec<Comment>;

// each comment is just a comment() + newline
pub fn comments<'t, I>() -> impl Parser<'t, I, Comments, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    comment()
        .separated_by(just(Token::Newline))
        .collect::<Vec<Comment>>()
}
