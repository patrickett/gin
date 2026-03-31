use crate::parse::delimited_list;
use crate::prelude::*;
use lexer::Token;

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
#[allow(clippy::large_enum_variant)]
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
    SelfRef(SimpleSpan),
    /// A capitalized variant constructor with arguments, e.g. `Some(5)`.
    TagCall(TagCall),
    /// A bare capitalized tag in expression position, e.g. `None`, `True`.
    AnonymousTag(Intern::<::std::string::String>, SimpleSpan),
    /// Stack-allocate an array: `(init_expr; N)` — emits `llvm.alloca N×sizeof(elem)`.
    TupleAlloc {
        init: Box<Spanned<Expr>>,
        size: usize,
    },
    /// Positional element read: `arr.N` — emits GEP + load.
    TupleGet {
        base: Box<Spanned<Expr>>,
        index: usize,
    },
    /// Positional element write: `arr.N: val` — emits GEP + store.
    TupleSet {
        base: Box<Spanned<Expr>>,
        index: usize,
        value: Box<Spanned<Expr>>,
    },
    /// Explicit numeric cast: `expr as Type` — emits trunci/extsi/sitofp/fptosi.
    Cast {
        expr: Box<Spanned<Expr>>,
        ty: Intern::<::std::string::String>,
    },
    /// Dynamic buffer element read: `buf.(i)` — emits GEP(i * elem_bytes) + load.
    BufGet {
        buf: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },
    /// Dynamic buffer element write: `buf.(i): val` — emits GEP(i * elem_bytes) + store.
    BufSet {
        buf: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
        value: Box<Spanned<Expr>>,
    },
    /// Take a raw pointer to a value: `@expr` — emits alloca + spill if needed, returns `!llvm.ptr`.
    TakePtr(Box<Spanned<Expr>>),
    /// Take a reference to a value: `^expr` — same layout as TakePtr for now.
    TakeRef(Box<Spanned<Expr>>),
    /// Dereference a pointer or reference: `*expr` — emits `llvm.load` of the pointed-to value.
    Deref(Box<Spanned<Expr>>),
    /// Unary negation: `-expr`.
    Negate(Box<Spanned<Expr>>),
    /// Tuple literal: `(e1, e2, …)` — at least two elements.
    TupleLit(Vec<Spanned<Expr>>),
}

pub fn expression<'t, I>() -> impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    recursive(|expr| {
        use Token::*;
        use chumsky::pratt::{infix, left, postfix, prefix};

        let inner = atom(expr.clone());

        let full_atom = choice((
            inner.boxed(),
            loop_expr(expr.clone())
                .map_with(|l, e| Spanned(Expr::Loop(l), e.span()))
                .boxed(),
            when_expr(expr.clone())
                .map_with(|w, e| Spanned(Expr::When(w), e.span()))
                .boxed(),
            if_expr(expr.clone())
                .map_with(|i, e| Spanned(Expr::If(i), e.span()))
                .boxed(),
        ));

        // Comparison operators (precedence 3)
        let comparison = infix(
            left(3),
            comparison_op(),
            |lhs: Spanned<Expr>, op: BinOp, rhs: Spanned<Expr>, extra| {
                Spanned(Expr::Binary(Binary::new(lhs, op, rhs)), extra.span())
            },
        );

        // Arithmetic operators (precedence 4)
        let arithmetic = infix(
            left(4),
            arithmetic_op(),
            |lhs: Spanned<Expr>, op: BinOp, rhs: Spanned<Expr>, extra| {
                Spanned(Expr::Binary(Binary::new(lhs, op, rhs)), extra.span())
            },
        );

        // Bitwise operators (precedence 3, same level as comparison — use parens to override)
        let bitwise = infix(
            left(3),
            bitwise_op(),
            |lhs: Spanned<Expr>, op: BinOp, rhs: Spanned<Expr>, extra| {
                Spanned(Expr::Binary(Binary::new(lhs, op, rhs)), extra.span())
            },
        );

        // Postfix tuple element read: `expr.N` (precedence 5, tightest)
        let tuple_get = postfix(
            5,
            just(Dot).ignore_then(select! { Token::Int(n) => n as usize }),
            |base: Spanned<Expr>, idx: usize, extra| {
                Spanned(
                    Expr::TupleGet {
                        base: Box::new(base),
                        index: idx,
                    },
                    extra.span(),
                )
            },
        );

        // Postfix cast: `expr as Type` (precedence 5, same as tuple_get)
        let cast = postfix(
            5,
            just(Token::As)
                .ignore_then(select! { Token::Tag(name) => Intern::<::std::string::String>::new(name.to_string()) }),
            |expr: Spanned<Expr>, ty: Intern::<::std::string::String>, extra| {
                Spanned(
                    Expr::Cast {
                        expr: Box::new(expr),
                        ty,
                    },
                    extra.span(),
                )
            },
        );

        // Postfix dynamic index read: `expr.(expr)` (precedence 5)
        let buf_get = postfix(
            5,
            just(Token::Dot)
                .ignore_then(just(Token::ParenOpen))
                .ignore_then(expr.clone())
                .then_ignore(just(Token::ParenClose)),
            |base: Spanned<Expr>, index: Spanned<Expr>, extra| {
                Spanned(
                    Expr::BufGet {
                        buf: Box::new(base),
                        index: Box::new(index),
                    },
                    extra.span(),
                )
            },
        );

        // Prefix unary negation: `-expr` (higher precedence than arithmetic)
        let negate = prefix(6, just(Token::Minus), |_, expr: Spanned<Expr>, extra| {
            Spanned(Expr::Negate(Box::new(expr)), extra.span())
        });

        full_atom
            .pratt((
                comparison, bitwise, arithmetic, tuple_get, cast, buf_get, negate,
            ))
            .padded_by(just(Newline).repeated())
    })
}

/// Base expression atoms — literals, format strings, function calls, and binds.
///
/// Does NOT include loops to prevent infinite recursion when used as the
/// sub-expression parser for bind and fn_call.
fn atom<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    // BufSet: `name.(index): value`  (must precede bind/fn_call)
    let buf_set = id_token()
        .then(
            just(Token::Dot)
                .ignore_then(just(Token::ParenOpen))
                .ignore_then(expr.clone())
                .then_ignore(just(Token::ParenClose)),
        )
        .then_ignore(just(Token::Colon))
        .then(expr.clone())
        .map_with(|((name, index), value), e| {
            Spanned(
                Expr::BufSet {
                    buf: Box::new(Spanned(
                        Expr::FnCall(FnCall {
                            path: ModPath::new(name, vec![], e.span()),
                            args: None,
                        }),
                        e.span(),
                    )),
                    index: Box::new(index),
                    value: Box::new(value),
                },
                e.span(),
            )
        })
        .boxed();

    // TupleSet: `name.N: value`  (must precede bind so `name` isn't consumed first)
    let tuple_set = id_token()
        .then_ignore(just(Token::Dot))
        .then(select! { Token::Int(n) => n as usize })
        .then_ignore(just(Token::Colon))
        .then(expr.clone())
        .map_with(|((name, idx), val), e| {
            Spanned(
                Expr::TupleSet {
                    base: Box::new(Spanned(
                        Expr::FnCall(FnCall {
                            path: ModPath::new(name, vec![], e.span()),
                            args: None,
                        }),
                        e.span(),
                    )),
                    index: idx,
                    value: Box::new(val),
                },
                e.span(),
            )
        })
        .boxed();

    // TupleLit: `(e1, e2, …)` with at least 2 comma-separated elements.
    let tuple_lit = just(Token::ParenOpen)
        .ignore_then(
            expr.clone()
                .separated_by(just(Token::Comma).then_ignore(just(Token::Newline).repeated()))
                .allow_trailing()
                .at_least(2)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::ParenClose))
        .map_with(|elems, e| Spanned(Expr::TupleLit(elems), e.span()))
        .boxed();

    // TupleAlloc: `(init_expr; N)`
    let tuple_alloc = just(Token::ParenOpen)
        .ignore_then(expr.clone())
        .then_ignore(just(Token::ColonSemi))
        .then(select! { Token::Int(n) => n as usize })
        .then_ignore(just(Token::ParenClose))
        .map_with(|(init, size), e| {
            Spanned(
                Expr::TupleAlloc {
                    init: Box::new(init),
                    size,
                },
                e.span(),
            )
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
        .map_with(|e, extra| Spanned(Expr::TakePtr(Box::new(e)), extra.span()))
        .boxed();

    // TakeRef: `^expr`
    let take_ref = just(Token::Caret)
        .ignore_then(expr.clone())
        .map_with(|e, extra| Spanned(Expr::TakeRef(Box::new(e)), extra.span()))
        .boxed();

    // Deref: `*expr`
    let deref = just(Token::Star)
        .ignore_then(expr.clone())
        .map_with(|e, extra| Spanned(Expr::Deref(Box::new(e)), extra.span()))
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
        literal()
            .map_with(|lit, e| Spanned(Expr::Lit(lit), e.span()))
            .boxed(),
        format_string(expr.clone())
            .map_with(|fs, e| Spanned(Expr::FormatString(fs), e.span()))
            .boxed(),
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
            .map_with(|(_, access), e| {
                Spanned(
                    match access {
                        None => Expr::SelfRef(e.span()),
                        Some((field, args)) => Expr::FnCall(FnCall {
                            path: ModPath::new(
                                Intern::<::std::string::String>::new("self".to_string()),
                                vec![field],
                                e.span(),
                            ),
                            args,
                        }),
                    },
                    e.span(),
                )
            })
            .boxed(),
        bind(expr.clone())
            .map_with(|b, e| Spanned(Expr::Bind(b), e.span()))
            .boxed(),
        fn_call(expr.clone())
            .map_with(|fc, e| Spanned(Expr::FnCall(fc), e.span()))
            .boxed(),
        // TagCall must come before AnonymousTag: `Some(5)` → TagCall, bare `None` → AnonymousTag.
        tag_call(expr.clone())
            .map_with(|tc, e| Spanned(Expr::TagCall(tc), e.span()))
            .boxed(),
        // Bare capitalized tag with no args (e.g. `None`, `True`, `False`).
        select! { Token::Tag(name) => Intern::<::std::string::String>::new(name.to_string()) }
            .map_with(|name, e| Spanned(Expr::AnonymousTag(name, e.span()), e.span()))
            .boxed(),
    ))
}
