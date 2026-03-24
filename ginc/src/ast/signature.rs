use crate::prelude::*;
use std::hash::{Hash, Hasher};

use super::{Bind, Declare, DeclareValue, DocComment, ParameterKind, Tag};

#[derive(Clone, Debug)]
pub struct FunctionSignature {
    pub name: IStr,
    pub params: Vec<ParamSignature>,
    pub doc_comment: Option<DocComment>,
    pub is_private: bool,
}

#[derive(Clone, Debug)]
pub struct TagSignature {
    pub name: IStr,
    pub value: TagValueSignature,
    pub doc_comment: Option<DocComment>,
    pub is_private: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParamSignature {
    Tagged(Tag),
    Generic,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TagValueSignature {
    Alias(Tag),
    Record(Vec<(IStr, ParamSignature)>),
    Range(i64, i64),
    InRange(i64, i64),
}

impl PartialEq for FunctionSignature {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.params == other.params
            && self.doc_comment == other.doc_comment
            && self.is_private == other.is_private
    }
}

impl Eq for FunctionSignature {}

impl Hash for FunctionSignature {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.params.hash(state);
        self.is_private.hash(state);
    }
}

impl PartialEq for TagSignature {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.value == other.value
            && self.doc_comment == other.doc_comment
            && self.is_private == other.is_private
    }
}

impl Eq for TagSignature {}

impl Hash for TagSignature {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.value.hash(state);
        self.is_private.hash(state);
    }
}

impl FunctionSignature {
    pub fn from_bind(bind: &Bind, doc: Option<&DocComment>, is_private: bool) -> Self {
        let params = bind
            .params()
            .as_ref()
            .map(|p| {
                p.iter()
                    .map(|(_, v)| match v {
                        ParameterKind::Tagged(tag) => ParamSignature::Tagged(tag.clone()),
                        ParameterKind::Generic => ParamSignature::Generic,
                        ParameterKind::Default(_) => ParamSignature::Default,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Self {
            name: bind.name(),
            params,
            doc_comment: doc.cloned(),
            is_private,
        }
    }
}

impl TagSignature {
    pub fn from_declare(decl: &Declare, doc: Option<&DocComment>, is_private: bool) -> Self {
        let value = match decl.value() {
            DeclareValue::Alias(tag) => TagValueSignature::Alias(tag.clone()),
            DeclareValue::Record(params) => {
                let fields = params
                    .iter()
                    .map(|(name, kind)| {
                        let sig = match kind {
                            ParameterKind::Tagged(t) => ParamSignature::Tagged(t.clone()),
                            ParameterKind::Generic => ParamSignature::Generic,
                            ParameterKind::Default(_) => ParamSignature::Default,
                        };
                        (*name, sig)
                    })
                    .collect();
                TagValueSignature::Record(fields)
            }
            DeclareValue::Range(r) => TagValueSignature::Range(r.start, r.end),
            DeclareValue::InRange(r) => TagValueSignature::InRange(r.start, r.end),
            DeclareValue::Set() => {
                TagValueSignature::Alias(Tag::Nominal(IStr::new("Set".to_string())))
            }
        };

        Self {
            name: decl.name(),
            value,
            doc_comment: doc.cloned(),
            is_private,
        }
    }
}
