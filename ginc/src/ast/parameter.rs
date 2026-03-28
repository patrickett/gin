use indexmap::IndexMap;

use crate::parse::delimited_list;
use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(clippy::large_enum_variant)]
pub enum ParameterKind {
    Generic,
    Tagged(Tag),
    Default(Spanned<Expr>),
}

impl std::fmt::Display for ParameterKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParameterKind::Generic => Ok(()),
            ParameterKind::Tagged(tag) => write!(f, " {}", tag),
            ParameterKind::Default(expr) => write!(f, ": {:?}", expr),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(clippy::large_enum_variant)]
pub enum ParamInfo {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Tag),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Spanned<Expr>),
}

// id Tag | Tag2
// id: expr -- note exprs cannot be | since this is actually an assignment/default value
// the expr however can return a Tag Union
pub fn parameter<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, (IStr, ParameterKind), ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = id_token();

    // Parse parameter with explicit handling of Tag tokens vs generic identifiers
    let param_info = choice((
        // Handle tag-based parameter typing: (p Person | User)
        tag.clone().map(ParamInfo::Tag),
        // Handle default parameter values: (p: 123)
        just(Token::Colon)
            .ignore_then(expr.clone())
            .map(ParamInfo::Default),
    ))
    .or_not();

    let named = id.then(param_info).map(|(name, info)| {
        let kind = match info {
            Some(info) => match info {
                ParamInfo::Tag(tag) => ParameterKind::Tagged(tag),
                ParamInfo::Default(expr) => ParameterKind::Default(expr),
            },
            None => ParameterKind::Generic,
        };
        (name, kind)
    });

    // Positional type argument: bare Tag with no name, e.g. `Ptr(Byte)`.
    // The tag name itself is used as the parameter key.
    let positional = tag.clone().map(|t: Tag| {
        let key = IStr::new(t.name().to_string());
        (key, ParameterKind::Tagged(t))
    });

    choice((named, positional))
}

pub type Parameters = IndexMap<IStr, ParameterKind>;

pub fn params<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Parameters, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    delimited_list(
        Token::ParenOpen,
        parameter(expr, tag),
        Token::Comma,
        Token::ParenClose,
    )
    .map(|pairs| pairs.into_iter().collect::<Parameters>())
}
