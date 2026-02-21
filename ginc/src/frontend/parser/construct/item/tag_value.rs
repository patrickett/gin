use crate::frontend::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct TagName(pub IStr);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagValue {
    // Tag2 ::= Tag1
    // PossibleTags ::= Tag1 | Tag2
    Alias(Tag),
    // Person ::= (name String, age Number)
    Record(Parameters),
    // Record(std::iter::Map<&'src str, Box<TagValue<'src>>>),
    // PersonSet ::= { p : Person }
    Set(/* TODO */),
    // Degree ::= 0..360
    Range(std::ops::Range<i64>),
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
            Self::Range(r) => {
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
    I: ValueInput<'t, Token = Token, Span = SimpleSpan>,
{
    let tag_name = select! { Token::Tag(name) => TagName(name) };

    let lhs = tag_name
        .then(params.clone().or_not())
        // TODO: maybe we replace Token::Is with `=`
        .then_ignore(choice((just(Token::Is), just(Token::IsReplacedBy))));

    // RHS: either a union of tags or a record
    let rhs = choice((
        tag(expr.clone()).map(TagValue::Alias),
        params.map(TagValue::Record),
        range().map(TagValue::Range),
    ));

    lhs.then(rhs)
        .map(|((tag_name, params), value)| Bind::Tag(tag_name, Params(params, value)))
}
