use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Assignment<'src> {
    Single {
        name: &'src str,
        // PERF: remove Vec<_> here
        params: Option<Vec<Parameter<'src>>>,
        expr: Box<Expr<'src>>,
    },
    Multiple {
        name: &'src str,
        params: Option<Vec<Parameter<'src>>>,
        exprs: Vec<Expr<'src>>,
        ret: Option<Box<Expr<'src>>>,
    },
}

pub fn assignment<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr<'s>, ParserError<'t, 's>> + Clone + 't,
    tag: impl Parser<'t, I, Tag<'s>, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Assignment<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => name };

    let params = parameter(expr.clone(), tag.clone())
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .or_not();

    // Single-line assignment
    let single = id
        .then(params.clone())
        .then_ignore(just(Token::Colon))
        .then(expr.clone())
        .map(|((name, params), rhs)| Assignment::Single {
            name,
            params,
            expr: Box::new(rhs),
        });

    // Multi-line assignment
    let multiple = id
        .then(params)
        .then_ignore(just(Token::Colon))
        .then_ignore(just(Token::Newline))
        .then_ignore(just(Token::Indent))
        // **Important**: use `expr.clone().repeated()` here instead of recursive call
        .then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::Dedent))
        .then_ignore(just(Token::Return))
        .then(expr.clone().or_not())
        // .then_ignore(just(Token::Dedent))
        .map(|(((name, params), exprs), ret)| Assignment::Multiple {
            name,
            params,
            exprs,
            ret: ret.map(Box::new),
        });

    choice((multiple, single))
}
