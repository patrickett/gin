use crate::HashFloat;
use i256::I256;
use std::fmt;

#[derive(Debug, Clone, Hash)]
#[allow(clippy::derived_hash_with_manual_eq)]
pub enum Literal {
    Number(usize),
    Float(HashFloat),
    Int(I256),
    String(String),
}

impl Eq for Literal {}

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

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::String(s) => write!(f, "'{s}'"),
            Literal::Number(n) => write!(f, "{n}"),
            Literal::Float(hf) => write!(f, "{}", hf.0),
            Literal::Int(n) => write!(f, "{n}"),
        }
    }
}
