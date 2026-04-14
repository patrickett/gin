use crate::span::SpanId;
use internment::Intern;

use crate::doc_comment::DocComment;
use crate::parameter::Parameters;
use crate::path::ModPath;
use crate::span::Spanned;
use crate::tag::Tag;

use crate::expr::Expr;

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

/// Lazily-formatted method name (e.g., "Single(a).method")
pub struct MethodName<'a> {
    // TODO: come up with a better name than receiver
    receiver: &'a Tag,
    name: Intern<String>,
}

impl std::fmt::Display for MethodName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.receiver {
            Tag::Nominal(type_name, _) => {
                write!(f, "{}.{}", type_name.as_str(), self.name.as_str())
            }
            Tag::Generic(type_name, params, _) => {
                write!(f, "{}(", type_name.as_str())?;
                for (i, (k, v)) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k.as_str(), v)?;
                }
                write!(f, ").{}", self.name.as_str())
            }
            Tag::Qualified(path) => {
                write!(f, "{}", path.root.as_str())?;
                for seg in &path.segments {
                    write!(f, ".{}", seg.as_str())?;
                }
                write!(f, ".{}", self.name.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    doc_comment: Option<DocComment>,
    attributes: BindAttributes,
    name: Intern<String>,
    pub name_span: SpanId,
    params: Option<Parameters>,
    value: BindValue,
    receiver_type: Option<Tag>,
    return_type_name: Option<Intern<String>>,
    /// Explicit capitalized return type annotation, e.g. `Str` in `foo() Str: expr`.
    pub return_tag: Option<Tag>,
    /// Explicit type annotation with value args, e.g. `Maybe(3)` in `val Maybe(3): Some(3)`.
    pub type_annotation: Option<(Intern<String>, Vec<Spanned<Expr>>)>,
    /// Qualified path for type annotation, e.g. `Maybe.Some` in `val Maybe.Some(3): ...`
    pub type_annotation_qual: Option<ModPath>,
    /// `true` for `:=` (immutable/const) binds; `false` for `:` (mutable, alloca-backed) binds.
    pub is_const: bool,
}

impl Bind {
    pub fn new(name: Intern<String>, name_span: SpanId, value: BindValue, is_const: bool) -> Self {
        Bind {
            doc_comment: None,
            attributes: BindAttributes::default(),
            name,
            name_span,
            params: None,
            value,
            receiver_type: None,
            return_type_name: None,
            return_tag: None,
            type_annotation: None,
            type_annotation_qual: None,
            is_const,
        }
    }

    pub fn with_return_type_name(mut self, name: Option<Intern<String>>) -> Self {
        self.return_type_name = name;
        self
    }

    pub fn return_type_name(&self) -> Option<&Intern<String>> {
        self.return_type_name.as_ref()
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_receiver_type(mut self, receiver_type: Option<Tag>) -> Self {
        self.receiver_type = receiver_type;
        self
    }

    pub fn name(&self) -> Intern<String> {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        self.doc_comment.as_ref()
    }

    pub fn with_attributes(mut self, attrs: BindAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn attributes(&self) -> &BindAttributes {
        &self.attributes
    }

    pub fn value(&self) -> &BindValue {
        &self.value
    }

    pub fn is_method(&self) -> bool {
        self.receiver_type.is_some()
    }

    pub fn receiver_type(&self) -> Option<&Tag> {
        self.receiver_type.as_ref()
    }

    pub fn method_name(&self) -> Option<MethodName<'_>> {
        self.receiver_type.as_ref().map(|t| MethodName {
            receiver: t,
            name: self.name,
        })
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.is_const.hash(state);
        self.receiver_type.hash(state);
        self.return_type_name.hash(state);
        // Hash params manually since HashMap doesn't impl Hash
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
