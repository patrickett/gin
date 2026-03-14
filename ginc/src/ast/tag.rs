//! Tags are almost synonymous with types in other languages.

use crate::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Variant {
    /// this comes from somewhere else its just one of the possible values
    /// holds its own doc comments
    External(Tag),
    /// defined within the current declare
    Local {
        doc_comment: Option<DocComment>,
        tag: Tag,
    },
}

impl Hash for Variant {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::External(tag) => tag.hash(state),
            Self::Local { doc_comment, tag } => {
                doc_comment.hash(state);
                tag.hash(state);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tag {
    Nominal(IStr),
    Generic(IStr, Parameters),
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tag::Nominal(name) => write!(f, "{}", name.as_str()),
            Tag::Generic(name, params) => {
                write!(f, "{}(", name.as_str())?;
                let mut first = true;
                for (k, v) in params {
                    if !first {
                        write!(f, ", ")?;
                    }
                    first = false;
                    write!(f, "{}: {}", k.as_str(), v)?;
                }
                write!(f, ")")
            }
        }
    }
}

impl Tag {
    pub fn name(&self) -> &str {
        match self {
            Tag::Nominal(name) => name.as_str(),
            Tag::Generic(name, _) => name.as_str(),
        }
    }
}

impl Hash for Tag {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Nominal(name) => name.hash(state),
            Self::Generic(name, params) => {
                name.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
    }
}

pub fn tag<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Tag, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|tag| {
        // Parse tag name (capitalized)
        let tag_name = select! { Token::Tag(name) => IStr::new(name.to_string()) };

        // Parse nominal or generic tag
        tag_name
            .then(params(expr.clone(), tag.clone()).or_not())
            .map(|(name, params)| match params {
                None => Tag::Nominal(name),
                Some(parameters) if parameters.is_empty() => Tag::Nominal(name),
                Some(parameters) => Tag::Generic(name, parameters),
            })
    })
}
