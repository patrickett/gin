use crate::{block, prelude::*};
use chumsky::span::SimpleSpan;
use lexer::Token;

/// For-in loop: iterate over a range or collection
///
/// Example:
/// ```gin
/// main:
///     for item in items
///     loop
/// return
/// ```
/// OR like a range
/// ```gin
/// main:
///     for i in 1...50
///     loop
/// return
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForInLoop {
    pub pat: Pattern,
    // TODO: check and make sure it accepts expression that can be iterated
    pub iter: Box<Spanned<Expr>>,
    pub exprs: Vec<Spanned<Expr>>,
}

pub fn for_loop_header_expr<'t, I>() -> impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;
    use chumsky::pratt::{infix, left};

    let atom = recursive(|expr| {
        choice((
            literal()
                .map_with(|lit, e| Spanned(Expr::Lit(lit), e.span()))
                .boxed(),
            fn_call(expr.clone())
                .map_with(|fc, e| Spanned(Expr::FnCall(fc), e.span()))
                .boxed(),
        ))
    });

    // Range operator (precedence 2)
    let range = infix(
        left(2),
        just(Infer),
        |lhs: Spanned<Expr>, _, rhs: Spanned<Expr>, extra| {
            Spanned(Expr::Range(Range::new(lhs, rhs)), extra.span())
        },
    );

    // Arithmetic operators (precedence 3)
    let arithmetic = infix(
        left(3),
        select! {
            Plus => BinOp::Add,
            Minus => BinOp::Subtract,
            Star => BinOp::Multiply,
            Slash => BinOp::Divide,
            Percent => BinOp::Modulo,
        },
        |lhs: Spanned<Expr>, op: BinOp, rhs: Spanned<Expr>, extra| {
            Spanned(Expr::Binary(Binary::new(lhs, op, rhs)), extra.span())
        },
    );

    atom.pratt((range, arithmetic))
        .padded_by(just(Newline).repeated())
}

pub fn for_in_loop<'t, I>(
    header_expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
    body_expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, ForInLoop, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let header = just(For)
        .ignore_then(pattern())
        .then_ignore(just(In))
        .then(header_expr.clone().map(Box::new));
    let body = body_expr.clone();
    let end = just(Token::Loop);

    block(header, body, end).map(|((pat, iter), exprs, _loop)| ForInLoop { pat, iter, exprs })
}
