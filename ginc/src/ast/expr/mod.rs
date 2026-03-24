use crate::parse::delimited_list;
use crate::prelude::*;

mod bind;
pub use bind::*;
pub mod format_string;
pub use format_string::*;
pub mod literal;
pub use literal::*;
mod import;
pub use import::*;
mod fn_call;
pub use fn_call::*;
mod tag_call;
pub use tag_call::*;
mod binary;
pub use binary::*;
pub mod r#loop;
pub use r#loop::{Loop as LoopEnum, *};
pub mod r#if;
pub use r#if::*;
pub mod range;
pub use range::*;
pub mod r#return;
pub use r#return::*;
pub mod when;
pub use when::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr {
    Loop(Loop),
    Binary(Binary),
    FnCall(FnCall),
    Lit(Literal),
    FormatString(FormatString),
    Range(Range),
    Bind(Bind),
    When(WhenExpr),
    If(IfExpr),
    SelfRef,
    /// A capitalized variant constructor with arguments, e.g. `Some(5)`.
    TagCall(TagCall),
    /// A bare capitalized tag in expression position, e.g. `None`, `True`.
    AnonymousTag(IStr),
    /// Stack-allocate an array: `(init_expr; N)` — emits `llvm.alloca N×sizeof(elem)`.
    TupleAlloc { init: Box<Expr>, size: usize },
    /// Positional element read: `arr.N` — emits GEP + load.
    TupleGet { base: Box<Expr>, index: usize },
    /// Positional element write: `arr.N: val` — emits GEP + store.
    TupleSet { base: Box<Expr>, index: usize, value: Box<Expr> },
    /// Explicit numeric cast: `expr as Type` — emits trunci/extsi/sitofp/fptosi.
    Cast { expr: Box<Expr>, ty: IStr },
    /// Dynamic buffer element read: `buf[i]` — emits GEP(i * elem_bytes) + load.
    BufGet { buf: Box<Expr>, index: Box<Expr> },
    /// Dynamic buffer element write: `buf[i]: val` — emits GEP(i * elem_bytes) + store.
    BufSet { buf: Box<Expr>, index: Box<Expr>, value: Box<Expr> },
    /// Take a raw pointer to a value: `@expr` — emits alloca + spill if needed, returns `!llvm.ptr`.
    TakePtr(Box<Expr>),
    /// Take a reference to a value: `^expr` — same layout as TakePtr for now.
    TakeRef(Box<Expr>),
    /// Dereference a pointer or reference: `*expr` — emits `llvm.load` of the pointed-to value.
    Deref(Box<Expr>),
    /// Unary negation: `-expr`.
    Negate(Box<Expr>),
    /// Tuple literal: `(e1, e2, …)` — at least two elements.
    TupleLit(Vec<Expr>),
}

pub fn expression<'t, I>() -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|expr| {
        use chumsky::pratt::{infix, left, postfix, prefix};
        use Token::*;

        let inner = atom(expr.clone());

        let full_atom = choice((
            inner.boxed(),
            loop_expr(expr.clone()).map(Expr::Loop).boxed(),
            when_expr(expr.clone()).map(Expr::When).boxed(),
            if_expr(expr.clone()).map(Expr::If).boxed(),
        ));

        // Comparison operators (precedence 3)
        let comparison = infix(
            left(3),
            comparison_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
        );

        // Arithmetic operators (precedence 4)
        let arithmetic = infix(
            left(4),
            arithmetic_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
        );

        // Bitwise operators (precedence 3, same level as comparison — use parens to override)
        let bitwise = infix(
            left(3),
            bitwise_op(),
            |lhs: Expr, op: BinOp, rhs: Expr, _| Expr::Binary(Binary::new(lhs, op, rhs)),
        );

        // Postfix tuple element read: `expr.N` (precedence 5, tightest)
        let tuple_get = postfix(
            5,
            just(Dot).ignore_then(select! { Token::Int(n) => n as usize }),
            |base: Expr, idx: usize, _| Expr::TupleGet {
                base: Box::new(base),
                index: idx,
            },
        );

        // Postfix cast: `expr as Type` (precedence 5, same as tuple_get)
        let cast = postfix(
            5,
            just(Token::As).ignore_then(select! { Token::Tag(name) => IStr::new(name.to_string()) }),
            |expr: Expr, ty: IStr, _| Expr::Cast {
                expr: Box::new(expr),
                ty,
            },
        );

        // Postfix dynamic index read: `expr[i]` (precedence 5)
        let buf_get = postfix(
            5,
            just(Token::BracketOpen)
                .ignore_then(expr.clone())
                .then_ignore(just(Token::BracketClose)),
            |base: Expr, index: Expr, _| Expr::BufGet {
                buf: Box::new(base),
                index: Box::new(index),
            },
        );

        // Prefix unary negation: `-expr` (higher precedence than arithmetic)
        let negate = prefix(
            6,
            just(Token::Minus),
            |_, expr: Expr, _| Expr::Negate(Box::new(expr)),
        );

        full_atom
            .pratt((comparison, bitwise, arithmetic, tuple_get, cast, buf_get, negate))
            .padded_by(just(Newline).repeated())
    })
}

/// Base expression atoms — literals, format strings, function calls, and binds.
///
/// Does NOT include loops to prevent infinite recursion when used as the
/// sub-expression parser for bind and fn_call.
fn atom<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    // BufSet: `name[index]: value`  (must precede bind/fn_call)
    let buf_set = id_token()
        .then(
            just(Token::BracketOpen)
                .ignore_then(expr.clone())
                .then_ignore(just(Token::BracketClose)),
        )
        .then_ignore(just(Token::Colon))
        .then(expr.clone())
        .map(|((name, index), value)| Expr::BufSet {
            buf: Box::new(Expr::FnCall(FnCall {
                path: ModPath::new(name, vec![]),
                args: None,
            })),
            index: Box::new(index),
            value: Box::new(value),
        })
        .boxed();

    // TupleSet: `name.N: value`  (must precede bind so `name` isn't consumed first)
    let tuple_set = id_token()
        .then_ignore(just(Token::Dot))
        .then(select! { Token::Int(n) => n as usize })
        .then_ignore(just(Token::Colon))
        .then(expr.clone())
        .map(|((name, idx), val)| Expr::TupleSet {
            base: Box::new(Expr::FnCall(FnCall {
                path: ModPath::new(name, vec![]),
                args: None,
            })),
            index: idx,
            value: Box::new(val),
        })
        .boxed();

    // TupleLit: `(e1, e2, …)` with at least 2 comma-separated elements.
    let tuple_lit = just(Token::ParenOpen)
        .ignore_then(
            expr.clone()
                .separated_by(
                    just(Token::Comma).then_ignore(just(Token::Newline).repeated()),
                )
                .allow_trailing()
                .at_least(2)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::ParenClose))
        .map(Expr::TupleLit)
        .boxed();

    // TupleAlloc: `(init_expr; N)`
    let tuple_alloc = just(Token::ParenOpen)
        .ignore_then(expr.clone())
        .then_ignore(just(Token::ColonSemi))
        .then(select! { Token::Int(n) => n as usize })
        .then_ignore(just(Token::ParenClose))
        .map(|(init, size)| Expr::TupleAlloc {
            init: Box::new(init),
            size,
        })
        .boxed();

    // Grouped expression: `(expr)` — explicit precedence grouping
    let paren_expr = just(Token::ParenOpen)
        .ignore_then(expr.clone())
        .then_ignore(just(Token::ParenClose))
        .boxed();

    // TakePtr: `@expr`
    let take_ptr = just(Token::At)
        .ignore_then(expr.clone())
        .map(|e| Expr::TakePtr(Box::new(e)))
        .boxed();

    // TakeRef: `^expr`
    let take_ref = just(Token::Caret)
        .ignore_then(expr.clone())
        .map(|e| Expr::TakeRef(Box::new(e)))
        .boxed();

    // Deref: `*expr`
    let deref = just(Token::Star)
        .ignore_then(expr.clone())
        .map(|e| Expr::Deref(Box::new(e)))
        .boxed();

    choice((
        buf_set,
        tuple_set,
        tuple_lit,
        tuple_alloc,
        paren_expr,
        take_ptr,
        take_ref,
        deref,
        literal().map(Expr::Lit).boxed(),
        format_string(expr.clone()).map(Expr::FormatString).boxed(),
        just(Token::SelfInstance)
            .then(
                just(Token::Dot)
                    .ignore_then(id_token())
                    .then(
                        delimited_list(
                            Token::ParenOpen,
                            expr.clone(),
                            Token::Comma,
                            Token::ParenClose,
                        )
                        .or_not(),
                    )
                    .or_not(),
            )
            .map(|(_, access)| match access {
                None => Expr::SelfRef,
                Some((field, args)) => Expr::FnCall(FnCall {
                    path: ModPath::new(IStr::new("self".to_string()), vec![field]),
                    args,
                }),
            })
            .boxed(),
        bind(expr.clone()).map(Expr::Bind).boxed(),
        fn_call(expr.clone()).map(Expr::FnCall).boxed(),
        // TagCall must come before AnonymousTag: `Some(5)` → TagCall, bare `None` → AnonymousTag.
        tag_call(expr.clone()).map(Expr::TagCall).boxed(),
        // Bare capitalized tag with no args (e.g. `None`, `True`, `False`).
        select! { Token::Tag(name) => Expr::AnonymousTag(IStr::new(name.to_string())) }.boxed(),
    ))
}
