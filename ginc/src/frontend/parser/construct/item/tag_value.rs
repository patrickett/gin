use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagName(pub String);

#[derive(Debug, Clone)]
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

pub fn tag_value<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    // // LHS-only parser: nominal or generic, but no union
    let tag_name = select! { Token::Tag(name) => TagName(name.to_string()) };

    let lhs = tag_name
        .then(params.clone().or_not())
        .then_ignore(choice((just(Token::Is), just(Token::IsReplacedBy))));

    // RHS: either a union of tags or a record
    let rhs = choice((
        tag(expr.clone()).map(TagValue::Alias),
        params.map(TagValue::Record),
        range().map(TagValue::Range),
    ));

    lhs.then(rhs)
        .map(|((tag_name, params), value)| Bind::Tag(tag_name, Params { params, value }))
}
