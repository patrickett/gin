//! Tags are almost synonymous with types in other languages.

use crate::frontend::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tag {
    Nominal(TagName),
    Generic(TagName, Parameters),
    Union { variants: Vec<Tag> },
}

impl Hash for Tag {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Nominal(name) => name.hash(state),
            Self::Generic(name, params) => {
                name.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
            Self::Union { variants } => variants.hash(state),
        }
    }
}

// TagName
// TagName(id)
// TagName(id AnotherTag(some_generic_value, num Number) | YetAnother)
pub fn tag<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Tag, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|tag| {
        // --- parse tag name (capitalized)
        let tag_name = select! { Token::Tag(name) => TagName(IStr::new(name.to_string())) };

        // --- nominal or generic tag
        let nominal_or_generic = tag_name
            .then(params(expr.clone(), tag.clone()).or_not())
            .map(|(name, params)| match params {
                None => Tag::Nominal(name),
                Some(parameters) if parameters.is_empty() => Tag::Nominal(name),
                Some(parameters) => Tag::Generic(name, parameters),
            })
            .boxed();

        // Separator between middle variants: Slashor + optional Newline + optional Indent
        let middle_sep = just(Token::SlashOr)
            .then_ignore(just(Token::Newline).or_not())
            .then_ignore(just(Token::Indent).or_not());

        // Trailing separator: optional, can be Bar, Newline, or nothing
        let trailing_sep = choice((just(Token::SlashOr), just(Token::Newline))).or_not();

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
