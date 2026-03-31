use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: Box<Spanned<Expr>>,
    pub end: Box<Spanned<Expr>>,
}

impl Range {
    pub fn new(start: Spanned<Expr>, end: Spanned<Expr>) -> Self {
        Self {
            start: Box::new(start),
            end: Box::new(end),
        }
    }
}

pub fn int_range<'t, I>() -> impl Parser<'t, I, std::ops::Range<i128>, ParserError<'t>> + Clone
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
