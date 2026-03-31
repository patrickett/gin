use crate::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Return(pub Option<Box<Spanned<Expr>>>);

pub fn r#return<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Return, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    // IMPORTANT: Order matters! More specific cases must come first.
    // with_value and anonymous_tag must come before bare because bare will
    // match 'Return' even if there are more tokens after it (due to .or_not()).

    // return <expr> - return with a value expression
    let with_value = just(Token::Return)
        .ignore_then(expr.clone())
        .map(|e| Return(Some(Box::new(e))));

    // return TagName - return creates an anonymous tag
    let anonymous_tag = just(Token::Return)
        .ignore_then(
            select! { Token::Tag(name) => Intern::<String>::new(name.to_string()) }
                .map_with(|name, e| (name, e.span())),
        )
        .then_ignore(just(Token::Newline).or_not())
        .map(|(name, span)| {
            Return(Some(Box::new(Spanned(
                Expr::AnonymousTag(name, span),
                span,
            ))))
        });

    // Bare return (no value) - must come last to avoid consuming Return
    // when there are more tokens. Accepts an optional newline (for files
    // without trailing newline).
    let bare = just(Token::Return)
        .then(just(Token::Newline).or_not().map(|_| ()))
        .to(Return(None));

    choice((with_value, anonymous_tag, bare))
}
