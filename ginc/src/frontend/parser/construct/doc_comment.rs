use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

// TODO: introduce doc comment lexer and parser for doc comments
#[derive(Debug, Clone)]
pub struct DocComment(pub String);

pub fn doc_comment<'t, 's: 't, I>() -> impl Parser<'t, I, DocComment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    select! { Token::DocComment(text) => text }
        .then_ignore(just(Token::Newline).repeated().or_not())
        .map(|c| {
            DocComment(
                c.strip_prefix("--- ")
                    .expect("removed doc comment prefix")
                    .to_string(),
            )
        })
}
