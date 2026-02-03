use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Literal {
    Number(usize),
    Float(f64),
    Int(i64),
    String(String),
    Ellipsis,
    Nothing,
}

pub fn literal<'tokens, 'src: 'tokens, I>(
) -> impl Parser<'tokens, I, Literal, ParserError<'tokens, 'src>>
where
    I: ValueInput<'tokens, Token = Token<'src>, Span = SimpleSpan>,
{
    use Token::*;
    select! {
        Nothing => Literal::Nothing,
        Ellipsis => Literal::Ellipsis,
        Int(n) => Literal::Int(n),
        Float(f) => Literal::Float(f),
        String(s) => Literal::String(s.to_string()),
    }
    .then_ignore(just(Newline).or_not())
}
