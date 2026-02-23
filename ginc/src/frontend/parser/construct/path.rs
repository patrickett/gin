use crate::frontend::prelude::*;

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
}

impl ModPath {
    /// Construct a new `ModPath` from an owned root and segment list.
    pub fn new(root: IStr, segments: Vec<IStr>) -> Self {
        Self { root, segments }
    }
}

/// Parser that consumes a dotted identifier sequence and produces an owned `Path`.
pub fn path<'t, I>() -> impl Parser<'t, I, ModPath, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = id_token();

    id.clone().then(
        just(Token::Dot)
            .ignore_then(id)
            .repeated()
            .collect::<Vec<IStr>>(),
    )
    .map(|(root, segs)| ModPath::new(root, segs))
}
