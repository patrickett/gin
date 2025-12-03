use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

// TODO: introduce doc comment lexer and parser for doc comments
#[derive(Debug, Clone)]
pub struct DocComment(pub String);

// A single DocComment can be spread across multiple lines
pub fn doc_comment<'t, 's: 't, I>() -> impl Parser<'t, I, DocComment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    select! { Token::DocComment(text) => text }
        .separated_by(just(Token::Newline))
        .collect::<Vec<_>>()
        .map(|c| {
            DocComment(
                c.into_iter()
                    .map(|s| s.strip_prefix("--- ").expect("removed doc comment prefix"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })
}
