use crate::parse::unescape::unescape;
use crate::prelude::*;
use chumsky::span::SimpleSpan;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormatPart {
    Text(String),
    Expr(Box<Spanned<Expr>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FormatString {
    pub parts: Vec<FormatPart>,
}

/// Parse a format string expression from the new token stream:
/// `FormatStringDelim [FormatStringText | FormatInterpStart expr FormatInterpEnd]* FormatStringDelim`
pub fn format_string<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, FormatString, ParserError<'t>> + Clone + 't
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let text_part = select! { Token::FormatStringText(s) => FormatPart::Text(unescape(s)) };

    let interp_part = just(Token::FormatInterpStart)
        .ignore_then(expr)
        .then_ignore(just(Token::FormatInterpEnd))
        .map(|e| FormatPart::Expr(Box::new(e)));

    just(Token::FormatStringDelim)
        .ignore_then(
            choice((text_part, interp_part))
                .repeated()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::FormatStringDelim))
        .map(|parts| FormatString { parts })
}
