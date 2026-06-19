use crate::expr::{Expr, Typed};
use crate::span::SpanId;
use internment::Intern;
use std::hash::{Hash, Hasher};

use crate::doc_comment::DocComment;
use crate::parameter::Parameters;
use crate::ty::Ty;

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

/// A trait implementation provided by a `Type.Trait(...)` declaration.
///
/// For example, in `Capacity.IsEmpty(is_empty: self.count > 0)`, this represents
/// the `IsEmpty(is_empty: self.count > 0)` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvidedTrait {
    pub trait_name: Intern<String>,
    pub trait_name_span: SpanId,
    /// Field definitions for the provided trait.
    /// e.g. `is_empty: self.count > 0` → `(is_empty, <expr>)`.
    pub fields: Vec<(Intern<String>, Typed<Expr>)>,
}

impl Hash for ProvidedTrait {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.trait_name.hash(state);
        self.trait_name_span.hash(state);
        for (k, v) in &self.fields {
            k.hash(state);
            v.hash(state);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    pub doc_comment: Option<DocComment>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    /// Full declaration site from the tag name through the `is`/`has` value.
    pub span: SpanId,
    pub params: Option<Parameters>,
    pub resolved_type: Option<Ty>,
    pub attributes: DeclareAttributes,
    pub value: DeclareValue,
    pub provided_traits: Vec<ProvidedTrait>,
}

impl Declare {
    pub fn new(name: Intern<String>, name_span: SpanId, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            name,
            name_span,
            span: name_span,
            params: None,
            resolved_type: None,
            attributes: DeclareAttributes::default(),
            value,
            provided_traits: Vec::new(),
        }
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn with_attributes(mut self, attrs: DeclareAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn with_provided_traits(mut self, traits: Vec<ProvidedTrait>) -> Self {
        self.provided_traits = traits;
        self
    }
}

impl Hash for Declare {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
        self.span.hash(state);
        match &self.params {
            None => 0u8.hash(state),
            Some(params) => {
                1u8.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
        self.value.hash(state);
        self.provided_traits.hash(state);
    }
}
