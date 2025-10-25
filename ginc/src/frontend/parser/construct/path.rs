use crate::frontend::{lexer::Token, parser::ParserError};
use chumsky::{Parser, input::ValueInput, prelude::*, select, span::SimpleSpan};

#[derive(Debug, Clone)]
// Paths are multi point identifiers. typically mapping to a function in a directory.
pub struct Path<'src> {
    /// The root of a path can be a name mapped in flask.json
    /// or is a child folder in the current directory.
    ///
    /// NOTE: If there is a name conflict it will error.
    root: &'src str,
    segments: Vec<&'src str>,
}

impl<'src> Path<'src> {
    pub fn new(root: &'src str, segments: Vec<&'src str>) -> Self {
        Self { root, segments }
    }
}

pub fn path<'t, 's: 't, I>() -> impl Parser<'t, I, Path<'s>, ParserError<'t, 's>>
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => name };

    id.then(
        just(Token::Dot)
            .ignore_then(id)
            .repeated()
            .collect::<Vec<&str>>(),
    )
    .map(|(root, segments)| Path::new(root, segments))
}
