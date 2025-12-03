use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Literal {
    Number(usize),
    Float(f64),
    Int(i64),
    String(String),
    Ellipsis,
}

pub fn literal<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Literal, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    use Token::*;
    select! {
        Ellipsis => Literal::Ellipsis,
        Int(n) => Literal::Int(n),
        Float(f) => Literal::Float(f),
    }
    .then_ignore(just(Newline).or_not())
}
