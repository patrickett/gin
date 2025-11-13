use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub struct Params<Value> {
    pub params: Option<Parameters>,
    /// typically the rhs of a `fn_name : {value}` or Tag is `{value}`
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefName(String);

#[derive(Debug, Clone)]
pub enum DefValue {
    Expr { expr: Box<Expr> },
    Body { exprs: Vec<Expr>, ret: Return },
}

// TODO:
// Ellipsis
// >>> ... = 'something'
// # SyntaxError: cannot assign to literal '...' (Ellipsis)

// TODO: support full function signature
// add(x Num, y Num) = Num: ...

#[derive(Debug, Clone)]

pub enum Bind {
    Tag(TagName, Params<TagValue>),
    Def(DefName, Params<DefValue>),
}

// TODO: can we use Map<ParameterName, ParameterValue> instead Vec<Parameter>
pub fn tag_value<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    // // LHS-only parser: nominal or generic, but no union
    let tag_name = select! { Token::Tag(name) => TagName(name.to_string()) };

    let lhs = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Is));
    // .map(|(name, parameters)| match parameters {
    //     None => Tag::Nominal(name),
    //     Some(parameters) if parameters.is_empty() => Tag::Nominal(name),
    //     Some(parameters) => Tag::Generic(name, parameters),
    // });

    // RHS: either a union of tags or a record
    let record = params;

    let rhs = choice((
        tag(expr.clone()).map(TagValue::Alias),
        record.map(TagValue::Record),
    ));

    lhs.then(rhs)
        .map(|((tag_name, params), value)| Bind::Tag(tag_name, Params { params, value }))
}

pub fn def_value<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let lhs = select! { Token::Id(name) => DefName(name.to_string()) }
        .then(params.or_not())
        .then_ignore(just(Token::Colon));

    // Single-line assignment
    let single = lhs.clone().then(expr.clone()).map(|((name, params), rhs)| {
        Bind::Def(
            name,
            Params {
                params,
                value: DefValue::Expr {
                    expr: Box::new(rhs),
                },
            },
        )
    });

    // Multi-line assignment
    let multiple = lhs
        .then_ignore(just(Token::Newline))
        .then_ignore(just(Token::Indent))
        .then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::Dedent))
        .then_ignore(just(Token::Newline).repeated().at_least(1).or_not())
        .then(r#return(expr.clone()))
        .map(|(((name, params), exprs), ret)| {
            Bind::Def(
                name,
                Params {
                    params,
                    value: DefValue::Body { exprs, ret },
                },
            )
        });

    choice((multiple, single))
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TagName(pub String);
