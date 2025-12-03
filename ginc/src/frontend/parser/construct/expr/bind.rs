use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct Params<Value> {
    pub params: Option<Parameters>,
    /// typically the rhs of a `fn_name : {value}` or Tag is `{value}`
    pub value: Value,
}

// TODO:
// Ellipsis
// >>> ... : 'something'
// # SyntaxError: cannot assign to literal '...' (Ellipsis)

#[derive(Debug, Clone)]
pub enum Bind {
    Tag(TagName, Params<TagValue>),
    Def(DefName, Params<DefValue>),
}

pub fn bind<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));
    let def_parser = def_value(expr.clone(), params.clone());
    let tag_parser = tag_value(expr, params);

    choice((def_parser, tag_parser))
}
