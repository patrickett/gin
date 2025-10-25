use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Parameter<'src> {
    Generic { name: &'src str },
    Tagged { name: &'src str, tag: Tag<'src> },
    Default { name: &'src str, expr: Expr<'src> },
}

#[derive(Debug, Clone)]
pub enum ParamInfo<'src> {
    /// Represents a type tag for the parameter, e.g. `(p Person)`.
    Tag(Tag<'src>),
    /// Represents a default value expression for the parameter, e.g. `(p: 123)`.
    Default(Expr<'src>),
}

// id Tag | Tag2
// id: expr -- note exprs cannot be | since this is actually an assignment/default value
// the expr however can return a Tag Union
pub fn parameter<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr<'s>, ParserError<'t, 's>> + Clone + 't,
    tag: impl Parser<'t, I, Tag<'s>, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Parameter<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => name };

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

    id.then(param_info).map(|(name, info)| match info {
        Some(info) => match info {
            ParamInfo::Tag(tag) => Parameter::Tagged { name, tag },
            ParamInfo::Default(expr) => Parameter::Default { name, expr },
        },
        None => Parameter::Generic { name },
    })
}

// single_tag
//     .separated_by(
//         just(Token::Bar).then_ignore(
//             // Allow an optional newline/indent after the bar
//             just(Token::Newline)
//                 .or_not()
//                 .then_ignore(just(Token::Indent).or_not()),
//         ),
//     )
//     .collect::<Vec<_>>()
//     .map(|variants| match variants.len() {
//         0 => panic!("`tag` parser received zero variants – this is a logic error"),
//         1 => variants.into_iter().next().unwrap(),
//         _ => Tag::Union { variants },
//     })
