//! Tags are almost synonymous with types in other languages.

use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Tag<'src> {
    Generic {
        name: &'src str,
        parameters: Vec<Parameter<'src>>,
    },
    Nominal {
        name: &'src str,
    },
    Union {
        variants: Vec<Tag<'src>>,
    },
}

// TagName
// TagName(id)
// TagName(id AnotherTag(some_generic_value, num Number) | YetAnother)
pub fn tag<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr<'s>, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Tag<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    recursive(|tag| {
        // --- parse tag name (capitalized)
        let tag_name = select! { Token::Tag(name) => name };

        // --- parse optional parameters inside parens
        let params = parameter(expr.clone(), tag.clone())
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
            .or_not();

        // --- nominal or generic tag
        let nominal_or_generic = tag_name
            .then(params)
            .map(|(name, params)| match params {
                None => Tag::Nominal { name },
                Some(parameters) if parameters.is_empty() => Tag::Nominal { name },
                Some(parameters) => Tag::Generic { name, parameters },
            })
            .boxed();

        // Separator between middle variants: Bar + optional Newline + optional Indent
        let middle_sep = just(Token::Bar)
            .then_ignore(just(Token::Newline).or_not())
            .then_ignore(just(Token::Indent).or_not());

        // Trailing separator: optional, can be Bar, Newline, or nothing
        let trailing_sep = choice((just(Token::Bar), just(Token::Newline))).or_not();

        // --- parse first variant + repeated remaining variants separated by `sep`
        nominal_or_generic
            .clone()
            .then(
                middle_sep
                    .ignore_then(nominal_or_generic.clone())
                    .repeated()
                    .collect::<Vec<_>>(),
            )
            .then_ignore(trailing_sep)
            .map(|(first, mut rest)| {
                rest.insert(0, first);
                rest
            })
            // --- consume the Dedent at the end of the multi-line union
            .then_ignore(just(Token::Dedent).or_not())
            // --- flatten into single union if multiple variants
            .map(|variants| match variants.len() {
                0 => panic!("Expected at least one tag variant"),
                1 => variants.into_iter().next().unwrap(),
                _ => Tag::Union { variants },
            })
    })
}
