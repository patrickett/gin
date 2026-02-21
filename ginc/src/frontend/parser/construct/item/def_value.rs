use crate::frontend::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct DefName(pub IStr);

impl DefName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(IStr::new(name.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for DefName {
    fn from(s: &str) -> Self {
        Self(IStr::new(s.to_string()))
    }
}

impl std::fmt::Display for DefName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DefValue {
    Expr(Box<Expr>),
    Body { exprs: Vec<Expr>, ret: Return },
}

pub fn def_value<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
    params: impl Parser<'t, I, Parameters, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token, Span = SimpleSpan>,
{
    use Token::*;

    let lhs = select! { Token::Id(name) => DefName(name) }
        .then(params.or_not())
        .then(tag(expr.clone()).or_not())
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
