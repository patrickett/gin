use crate::frontend::prelude::*;

#[derive(Debug, Clone)]
pub enum Bind {
    Single {
        doc_comment: Option<DocComment>,
        name: String,
        // PERF: remove Vec<_> here
        params: Option<Vec<Parameter>>,
        expr: Box<Expr>,
    },
    Body {
        doc_comment: Option<DocComment>,
        name: String,
        params: Option<Vec<Parameter>>,
        exprs: Vec<Expr>,
        ret: Return,
    },
}

// TODO:
// Ellipsis
// >>> ... = 'something'
// # SyntaxError: cannot assign to literal '...' (Ellipsis)

// TODO: support full function signature
// add(x Num, y Num) = Num: ...

pub fn bind<'t, 's: 't, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t, 's>> + Clone + 't,
    tag: impl Parser<'t, I, Tag, ParserError<'t, 's>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let params = parameter(expr.clone(), tag.clone())
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::ParenOpen), just(Token::ParenClose))
        .or_not();

    let lhs = doc_comment().or_not().then(
        select! { Token::Id(name) => name }
            .then(params)
            .then_ignore(just(Token::Colon)),
    );

    // Single-line assignment
    let single = lhs
        .clone()
        .then(expr.clone())
        .map(|((doc_comment, (name, params)), rhs)| Bind::Single {
            doc_comment,
            name: name.to_string(),
            params,
            expr: Box::new(rhs),
        });

    // Multi-line assignment
    let multiple = lhs
        .then_ignore(just(Token::Newline))
        .then_ignore(just(Token::Indent))
        .then(expr.clone().repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::Dedent))
        .then_ignore(just(Token::Newline).repeated().at_least(1).or_not())
        .then(r#return(expr.clone()))
        .map(|(((doc_comment, (name, params)), exprs), ret)| Bind::Body {
            name: name.to_string(),
            doc_comment,
            params,
            exprs,
            ret,
        });

    choice((multiple, single))
}
