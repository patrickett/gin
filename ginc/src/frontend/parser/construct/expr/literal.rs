use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Literal {
    Number(usize),
    Float(f64),
    Int(i64),
    String(String),
}

pub fn literal<'tokens, 'src: 'tokens, I>()
-> impl Parser<'tokens, I, Literal, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    select! {
        Token::Int(n) => Literal::Int(n),
        Token::Float(f) => Literal::Float(f),
    }
    .then_ignore(just(Token::Newline).or_not())
}
