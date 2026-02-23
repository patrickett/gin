use crate::frontend::prelude::*;

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
    let int = select! { Token::Int(int) => int };

    int.then_ignore(just(Token::Infer))
        .then(int)
        .map(|(start, end)| std::ops::Range { start, end })
}
