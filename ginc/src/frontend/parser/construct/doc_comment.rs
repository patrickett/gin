use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*, span::SimpleSpan, Parser};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Documented<Item: PartialEq + Eq + std::hash::Hash> {
    pub doc: Option<DocComment>,
    /// Should only ever be a Tag or Def
    pub item: Item,
}

// TODO: Implement doc comment lexer and parser support
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocComment(pub String);

// A single DocComment can be spread across multiple lines
pub fn doc_comment<'t, I>() -> impl Parser<'t, I, DocComment, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::DocComment(text) => text }
        .separated_by(just(Token::Newline))
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|c: Vec<&str>| {
            DocComment(
                c.into_iter()
                    .map(|s| {
                        s.strip_prefix("--- ")
                            .expect("removed doc comment prefix")
                            .to_owned()
                    })
                    .collect::<Vec<String>>()
                    .join("\n"),
            )
        })
}
