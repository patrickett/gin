use std::collections::HashMap;

use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum ParameterKind {
    Generic,
    Tagged(Tag),
    Default(Expr),
}

#[derive(Debug, Clone)]
pub enum ParamInfo {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Tag),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Expr),
}

// id Tag | Tag2
// id: expr -- note exprs cannot be | since this is actually an assignment/default value
// the expr however can return a Tag Union
pub fn parameter<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, (ParamName, ParameterKind), ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => name.to_string() };

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

pub type ParamName = String;
pub type Parameters = HashMap<ParamName, ParameterKind>;

pub fn params<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Parameters, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    parameter(expr.clone(), tag.clone())
        .separated_by(just(Token::Comma))
        // .allow_trailing()
        .collect::<Parameters>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
}
