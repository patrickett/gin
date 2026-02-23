use crate::frontend::{parser::block, prelude::*};
use std::hash::Hash;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Expr>),
    Body { exprs: Vec<Expr>, ret: Return },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    doc_comment: Option<DocComment>,
    name: IStr,
    params: Option<Parameters>,
    value: BindValue,
}

impl Bind {
    pub fn new(name: IStr, value: BindValue) -> Self {
        Bind {
            name,
            value,
            params: None,
            doc_comment: None,
        }
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn name(&self) -> IStr {
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

    pub fn value(&self) -> &BindValue {
        &self.value
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
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

pub fn bind<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));

    use Token::*;

    let lhs = id_token()
        .then(params.or_not())
        .then(tag(expr.clone()).or_not())
        .then_ignore(just(Token::Colon));

    let single = lhs
        .clone()
        .then(expr.clone())
        .map(|(((name, params), _opt_tag), rhs)| {
            // TODO: do something with optional return type
            Bind::new(name, BindValue::Expr(Box::new(rhs))).with_params(params)
        });

    let multiple = lhs
        .then(block(
            just(Newline),          // header
            expr.clone(),           // body
            r#return(expr.clone()), // closer
        ))
        .map(|(((name, params), _opt_tag), (_nl, exprs, ret))| {
            Bind::new(name, BindValue::Body { exprs, ret }).with_params(params)
        });

    let bind = choice((multiple, single));

    doc_comment()
        .or_not()
        .then(bind)
        .map(|(doc, bind)| {
            let doc = doc.and_then(|d| if d.0.is_empty() { None } else { Some(d) });
            bind.with_doc(doc)
        })
}
