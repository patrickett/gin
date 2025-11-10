// ginc/src/frontend/parser/construct/path.rs
use crate::frontend::{lexer::Token, parser::ParserError};
use chumsky::{input::ValueInput, prelude::*, select, span::SimpleSpan};

/// A multi‑segment identifier.
///
/// `root` and every element of `segments` are **owned** strings.
/// This removes the need for a `'src` lifetime and allows the type to be
/// stored in caches or sent across threads safely.
#[derive(Debug, Clone)]
pub struct Path {
    /// The root of a path can be a name mapped in `flask.json` or
    /// a child folder in the current directory.
    ///
    /// NOTE: If there is a name conflict it will error.
    pub root: String,
    pub segments: Vec<String>,
}

impl Path {
    /// Construct a new `Path` from an owned root and segment list.
    pub fn new(root: String, segments: Vec<String>) -> Self {
        Self { root, segments }
    }
}

/// Parser that consumes a dotted identifier sequence and produces an owned `Path`.
///
/// The parser still works over the same token stream (`Token<'s>`), but it
/// collects the identifiers as `&str` and then converts them to `String`
/// before constructing the `Path`.
pub fn path<'t, 's: 't, I>() -> impl Parser<'t, I, Path, ParserError<'t, 's>>
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    // Grab the first identifier (the root).
    let id = select! { Token::Id(name) => name };

    // Grab zero or more “. <ident>” sequences.
    id.then(
        just(Token::Dot)
            .ignore_then(id)
            .repeated()
            .collect::<Vec<&str>>(),
    )
    // Convert the slice references into owned strings.
    .map(|(root, segs)| {
        Path::new(
            root.to_string(),
            segs.into_iter().map(|s| s.to_string()).collect(),
        )
    })
}
