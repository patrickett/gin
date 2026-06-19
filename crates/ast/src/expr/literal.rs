use crate::HashFloat;
use std::fmt;

#[derive(Debug, Clone, Hash)]
pub enum Literal {
    Number(usize),
    Float(HashFloat),
    Int(u128),
    String(String),
}

impl PartialEq for Literal {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float(HashFloat(a)), Self::Float(HashFloat(b))) => a.to_bits() == b.to_bits(),
            (Self::Number(a), Self::Number(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::String(a), Self::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Literal {}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Number(n) => write!(f, "{n}"),
            Literal::Float(v) => write!(f, "{v}"),
            Literal::Int(n) => write!(f, "{n}"),
            Literal::String(s) => write!(f, "'{s}'"),
        }
    }
}
