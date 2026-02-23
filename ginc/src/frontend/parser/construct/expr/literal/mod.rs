use crate::frontend::prelude::*;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub enum Literal {
    Number(usize),
    Float(f64),
    Int(i64),
    String(String),
    Nothing,
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Nothing, Self::Nothing) => true,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl Hash for Literal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Float(f) => f.to_bits().hash(state),
            Self::Number(n) => n.hash(state),
            Self::Int(i) => i.hash(state),
            Self::String(s) => s.hash(state),
            Self::Nothing => {}
        }
    }
}

pub fn literal<'t, I>() -> impl Parser<'t, I, Literal, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let valid = select! {
        Token::Int(n) => Literal::Int(n),
        Token::Float(f) => Literal::Float(f),
        Token::String(s) => Literal::String(s.to_string()),
    };

    // Accept unterminated strings for error recovery — the diagnostic
    // is reported from the tokenization step with the real byte span.
    let unclosed_string = select! {
        Token::UnterminatedString(s) => Literal::String(s.to_string()),
    };

    valid
        .or(unclosed_string)
        .then_ignore(just(Token::Newline).or_not())
}
