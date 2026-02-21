use indexmap::IndexMap;

use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParameterKind {
    Generic,
    Tagged(Tag),
    Default(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamInfo {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Tag),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Expr),
}

// id Tag | Tag2
// id: expr -- note exprs cannot be | since this is actually an assignment/default value
// the expr however can return a Tag Union
pub fn parameter<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, (ParamName, ParameterKind), ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => IStr::new(name.to_string()) };

    // Parse parameter with explicit handling of Tag tokens vs generic identifiers
    let param_info = choice((
        // Handle tag-based parameter typing: (p Person | User)
        tag.map(ParamInfo::Tag),
        // Handle default parameter values: (p: 123)
        just(Token::Colon)
            .ignore_then(expr.clone())
            .map(ParamInfo::Default),
    ))
    .or_not();

    id.then(param_info).map(|(name, info)| {
        let kind = match info {
            Some(info) => match info {
                ParamInfo::Tag(tag) => ParameterKind::Tagged(tag),
                ParamInfo::Default(expr) => ParameterKind::Default(expr),
            },
            None => ParameterKind::Generic,
        };

        (name, kind)
    })
}

pub type ParamName = IStr;
pub type Parameters = IndexMap<ParamName, ParameterKind>;

pub fn params<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Parameters, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    parameter(expr.clone(), tag.clone())
        .separated_by(just(Token::Comma))
        // .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .map(|pairs| pairs.into_iter().collect::<Parameters>())
}
