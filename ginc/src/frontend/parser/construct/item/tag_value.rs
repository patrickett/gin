use crate::frontend::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TagName(pub IStr);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagValue {
    Alias(Tag),
    Record(Parameters),
    Set(/* TODO */),
    Range(std::ops::Range<i64>),
    // DiceThrow is in 1...6 (element of range)
    InRange(std::ops::Range<i64>),
}

impl Hash for TagValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Alias(tag) => tag.hash(state),
            Self::Record(params) => {
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
            Self::Set() => {}
            Self::Range(r) | Self::InRange(r) => {
                r.start.hash(state);
                r.end.hash(state);
            }
        }
    }
}

pub fn tag_value<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let tag_name = select! { Token::Tag(name) => TagName(IStr::new(name.to_string())) };

    let lhs_has = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Has));

    let lhs_is = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Is));

    let rhs_record = choice((
        tag(expr.clone()).map(TagValue::Alias),
        params.map(TagValue::Record),
    ));

    let rhs_union_or_range = choice((
        just(Token::In).ignore_then(range()).map(TagValue::InRange),
        range().map(TagValue::Range),
        tag(expr.clone()).map(TagValue::Alias),
    ));

    choice((lhs_has.then(rhs_record), lhs_is.then(rhs_union_or_range)))
        .map(|((tag_name, params), value)| Bind::Tag(tag_name, Params(params, value)))
}
