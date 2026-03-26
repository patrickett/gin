use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Expr>,
    pub end: Box<Expr>,
}

impl Range {
    pub fn new(start: Expr, end: Expr) -> Self {
        let start = Box::new(start);
        let end = Box::new(end);
        Self { start, end }
    }
}

pub fn int_range<'t, I>() -> impl Parser<'t, I, std::ops::Range<i64>, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let signed_int = just(Token::Minus)
        .or_not()
        .then(select! { Token::Int(n) => n })
        .map(|(neg, n)| if neg.is_some() { -n } else { n });

    signed_int
        .then_ignore(just(Token::Infer))
        .then(signed_int)
        .map(|(start, end)| std::ops::Range { start, end })
}
