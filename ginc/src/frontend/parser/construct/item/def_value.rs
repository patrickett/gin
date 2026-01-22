use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefName(String);

#[derive(Debug, Clone)]
pub enum DefValue {
    Expr(Box<Expr>),
    Body { exprs: Vec<Expr>, ret: Return },
}

pub fn def_value<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    use Token::*;

    let lhs = select! { Token::Id(name) => DefName(name.to_string()) }
        .then(params.or_not())
        .then(
            just(Token::ArrowRight)
                .ignore_then(tag(expr.clone()))
                .or_not(),
        )
        .then_ignore(just(Token::Colon));

    let single = lhs
        .clone()
        .then(expr.clone())
        .map(|(((name, params), _opt_tag), rhs)| {
            // TODO: do something with optional return type
            Bind::Def(name, Params(params, DefValue::Expr(Box::new(rhs))))
        });

    let multiple = lhs
        .then(block(
            just(Newline),          // header
            expr.clone(),           // body
            r#return(expr.clone()), // closer
        ))
        .map(|(((name, params), _opt_tag), (_nl, exprs, ret))| {
            Bind::Def(name, Params(params, DefValue::Body { exprs, ret }))
        });

    choice((multiple, single))
}
