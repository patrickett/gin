use crate::prelude::*;

/// A multi‑segment identifier.
///
/// `root` and every element of `segments` are **owned** strings.
/// This removes the need for a `'src` lifetime and allows the type to be
/// stored in caches or sent across threads safely.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModPath {
    /// The root of a path can be a name mapped in `flask.json` or
    /// a child folder in the current directory.
    ///
    /// NOTE: If there is a name conflict it will error.
    pub root: IStr,
    pub segments: Vec<IStr>,
    /// Source location for diagnostic reporting.
    pub span: SimpleSpan,
}

// TODO: move SimpleSpan out of ModPath and wrap Spanned<ModPath>

impl ModPath {
    /// Construct a new `ModPath` from an owned root and segment list.
    pub fn new(root: IStr, segments: Vec<IStr>, span: SimpleSpan) -> Self {
        Self {
            root,
            segments,
            span,
        }
    }
}

/// Parser that consumes a dotted identifier sequence and produces an owned `Path`.
pub fn path<'t, I>() -> impl Parser<'t, I, ModPath, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = id_token();

    id.clone()
        .then(
            just(Token::Dot)
                .ignore_then(id)
                .repeated()
                .collect::<Vec<IStr>>(),
        )
        .map_with(|(root, segs), e| ModPath::new(root, segs, e.span()))
}

/// Parser that consumes a Tag-rooted dotted path (e.g. `Byte.new`, `Int.to_string`).
///
/// This enables calling static/impl methods directly on a type name:
/// `Tag.method` or `Tag.method.sub`
pub fn tag_path<'t, I>() -> impl Parser<'t, I, ModPath, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Tag(name) => IStr::new(name.to_string()) }
        .then(
            just(Token::Dot)
                .ignore_then(id_token())
                .repeated()
                .at_least(1)
                .collect::<Vec<IStr>>(),
        )
        .map_with(|(root, segs), e| ModPath::new(root, segs, e.span()))
}

/// Parser for qualified variant constructor paths (e.g. `Maybe.Some`, `Result.Ok`).
///
/// Matches `Tag.Tag` patterns where both the root and segments are capitalized Tags.
pub fn tag_variant_path<'t, I>() -> impl Parser<'t, I, ModPath, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    select! { Token::Tag(name) => IStr::new(name.to_string()) }
        .then(
            just(Token::Dot)
                .ignore_then(select! { Token::Tag(name) => IStr::new(name.to_string()) })
                .repeated()
                .at_least(1)
                .collect::<Vec<IStr>>(),
        )
        .map_with(|(root, segs), e| ModPath::new(root, segs, e.span()))
}
