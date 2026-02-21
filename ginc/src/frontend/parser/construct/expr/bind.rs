use crate::frontend::prelude::*;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Params<Value: PartialEq + Eq>(
    pub Option<Parameters>,
    /// typically the rhs of a `fn_name : {value}` or Tag is `{value}`
    pub Value,
);

impl<Value: PartialEq + Eq + Hash> Hash for Params<Value> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match &self.0 {
            None => 0u8.hash(state),
            Some(params) => {
                1u8.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
        self.1.hash(state);
    }
}

// TODO:
// Ellipsis
// >>> ... : 'something'
// # SyntaxError: cannot assign to literal '...' (Ellipsis)

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Bind {
    Tag(TagName, Params<TagValue>),
    Def(DefName, Params<DefValue>),
}

pub fn bind<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));
    let def_parser = def_value(expr.clone(), params.clone());
    let tag_parser = tag_value(expr, params);

    choice((def_parser, tag_parser))
}
