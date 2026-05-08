use crate::span::SpanId;
use internment::Intern;
use std::hash::{Hash, Hasher};

use crate::doc_comment::DocComment;
use crate::parameter::Parameters;

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    doc_comment: Option<DocComment>,
    attributes: DeclareAttributes,
    name: Intern<String>,
    pub name_span: SpanId,
    params: Option<Parameters>,
    value: DeclareValue,
}

impl Declare {
    pub fn new(name: Intern<String>, name_span: SpanId, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            attributes: DeclareAttributes::default(),
            name,
            name_span,
            params: None,
            value,
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

    pub fn doc_comment(&self) -> Option<&DocComment> {
        self.doc_comment.as_ref()
    }

    pub fn name(&self) -> Intern<String> {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn value(&self) -> &DeclareValue {
        &self.value
    }

    pub fn attributes(&self) -> &DeclareAttributes {
        &self.attributes
    }

    pub fn with_attributes(mut self, attrs: DeclareAttributes) -> Self {
        self.attributes = attrs;
        self
    }
}

impl std::fmt::Display for Declare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name.as_str())?;
        if let Some(params) = &self.params {
            write!(f, "[")?;
            let mut first = true;
            for (k, v) in params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}{v}", k.as_str())?;
            }
            write!(f, "]")?;
        }
        let keyword = match &self.value {
            DeclareValue::Record(_) => " has",
            _ => " is",
        };
        write!(f, "{keyword} {}", self.value)
    }
}

impl Hash for Declare {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
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
    }
}
