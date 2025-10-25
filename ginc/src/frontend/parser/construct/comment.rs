use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

pub fn comment<'t, 's: 't, I>() -> impl Parser<'t, I, Expr<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    // Matches: # ... newline | #
    just(Token::Pound)
        .ignore_then(none_of([Token::Newline, Token::Pound]).repeated())
        .then_ignore(one_of([Token::Newline, Token::Pound]).or_not())
        .map(|tokens: Vec<Token<'s>>| {
            // Convert all intermediate tokens into a reconstructed string slice
            let mut text_parts = Vec::new();
            for token in tokens {
                match token {
                    Token::Id(s)
                    | Token::Type(s)
                    | Token::Str(s)
                    | Token::Number(s)
                    | Token::Other(s) => text_parts.push(s),
                    _ => {}
                }
            }
            Expr::Comment(&text_parts.join(" "))
        })
}
