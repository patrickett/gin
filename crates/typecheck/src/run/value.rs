//! The values a program holds while it runs.

use std::fmt;
use std::sync::Arc;

use i256::I256;

/// Which nominal type a record is built from, named by where it was declared so that every
/// module agrees on it (layout numbers are per module). Type arguments are not part of it: a
/// match never mixes two applications of one generic type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Tag {
    pub home: Arc<str>,
    pub name: Arc<str>,
}

impl Tag {
    /// The name a program writes: a member of a sum is declared `Sum.Member` but written
    /// `Member`.
    pub fn short(&self) -> &str {
        self.name.rsplit('.').next().unwrap_or(&self.name)
    }

    /// The sum a member belongs to, when this tag is a sum member.
    pub fn sum(&self) -> Option<&str> {
        self.name.rsplit_once('.').map(|(sum, _)| sum)
    }
}

/// A value of a Gin program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Unit,
    /// An exact integer. The checker proves every integer inside its domain.
    Integer(I256),
    /// Compile-time text.
    Text(String),
    /// A record, a member of a sum (its payload is its fields), or a named domain (one field).
    Record { tag: Tag, fields: Vec<Value> },
}

impl Value {
    pub fn integer(value: i128) -> Self {
        Self::Integer(I256::from(value))
    }

    /// The integer inside, when this is one.
    pub fn as_integer(&self) -> Option<I256> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit => write!(out, "()"),
            Self::Integer(value) => write!(out, "{value}"),
            Self::Text(text) => write!(out, "{text:?}"),
            Self::Record { tag, fields } if fields.is_empty() => write!(out, "{}", tag.short()),
            Self::Record { tag, fields } => {
                write!(out, "{}(", tag.short())?;
                for (position, field) in fields.iter().enumerate() {
                    if position > 0 {
                        write!(out, ", ")?;
                    }
                    write!(out, "{field}")?;
                }
                write!(out, ")")
            }
        }
    }
}
