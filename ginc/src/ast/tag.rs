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

impl Variant {
    pub fn tag(&self) -> &Tag {
        match self {
            Variant::External(tag) => tag,
            Variant::Local { tag, .. } => tag,
        }
    }
}

impl std::fmt::Display for Variant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.tag())
    }
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
    Qualified(ModPath),
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
                    write!(f, "{}{v}", k.as_str())?;
                }
                write!(f, ")")
            }
            Tag::Qualified(path) => {
                write!(f, "{}", path.root.as_str())?;
                for seg in &path.segments {
                    write!(f, ".{}", seg.as_str())?;
                }
                Ok(())
            }
        }
    }
}

impl Tag {
    pub fn name(&self) -> &str {
        match self {
            Tag::Nominal(name) => name.as_str(),
            Tag::Generic(name, _) => name.as_str(),
            Tag::Qualified(path) => path.segments.last().map(|s| s.as_str()).unwrap_or(path.root.as_str()),
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
            Self::Qualified(path) => {
                path.root.hash(state);
                path.segments.hash(state);
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
        // Qualified type: Bool.True, Maybe.Some
        let qualified = super::tag_variant_path()
            .then(params(expr.clone(), tag.clone()).or_not())
            .map(|(path, params)| {
                match params {
                    None => Tag::Qualified(path),
                    Some(parameters) if parameters.is_empty() => Tag::Qualified(path),
                    Some(parameters) => {
                        // For generics with qualified paths, use the last segment as the name
                        let name = path.segments.last().copied().unwrap_or(path.root);
                        Tag::Generic(name, parameters)
                    }
                }
            })
            .boxed();

        // Simple tag name (capitalized)
        let tag_name = select! { Token::Tag(name) => IStr::new(name.to_string()) };

        // Parse nominal or generic tag
        let simple = tag_name
            .then(params(expr.clone(), tag.clone()).or_not())
            .map(|(name, params)| match params {
                None => Tag::Nominal(name),
                Some(parameters) if parameters.is_empty() => Tag::Nominal(name),
                Some(parameters) => Tag::Generic(name, parameters),
            })
            .boxed();

        // Prefer qualified to avoid ambiguity
        choice((qualified, simple))
    })
}
