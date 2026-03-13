use crate::prelude::*;
use std::hash::{Hash, Hasher};

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    doc_comment: Option<DocComment>,
    attributes: DeclareAttributes,
    name: IStr,
    params: Option<Parameters>,
    value: DeclareValue,
}

impl Declare {
    pub fn new(name: IStr, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            attributes: DeclareAttributes::default(),
            name,
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

    pub fn name(&self) -> IStr {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn value(&self) -> &DeclareValue {
        &self.value
    }
}

impl Hash for Declare {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
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

pub fn declare<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Declare, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));
    let tag_name = select! { Token::Tag(name) => IStr::new(name.to_string()) };

    let lhs_has = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Has));

    let lhs_is = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Is));

    let rhs_record = choice((
        tag(expr.clone()).map(DeclareValue::Alias),
        params.map(DeclareValue::Record),
    ));

    let rhs_union_or_range = choice((
        just(Token::In)
            .ignore_then(int_range())
            .map(DeclareValue::InRange),
        int_range().map(DeclareValue::Range),
        tag(expr.clone()).map(DeclareValue::Alias),
    ));

    let decl = choice((lhs_has.then(rhs_record), lhs_is.then(rhs_union_or_range)))
        .map(|((tag_name, params), value)| Declare::new(tag_name, value).with_params(params));

    doc_comment().or_not().then(decl).map(|(doc, decl)| {
        let doc = doc.and_then(|d| if d.0.is_empty() { None } else { Some(d) });
        decl.with_doc(doc)
    })
}
