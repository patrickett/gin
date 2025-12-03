use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefName(String);

#[derive(Debug, Clone)]
pub enum DefValue {
    Expr { expr: Box<Expr> },
    Body { exprs: Vec<Expr>, ret: Return },
}

// TODO: support full function signature
// add(x Num, y Num) = Num: ...
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
        .then_ignore(just(Newline))
        .then(
            expr.clone()
                .or_not()
                .delimited_by(just(Indent), just(Dedent).then(just(Newline)).or_not())
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then(r#return(expr.clone()))
        .map(|(((name, params), exprs), ret)| {
            Bind::Def(
                name,
                Params {
                    params,
                    value: DefValue::Body {
                        exprs: exprs.into_iter().flatten().collect(),
                        ret,
                    },
                },
            )
        });

    choice((multiple, single))
}
