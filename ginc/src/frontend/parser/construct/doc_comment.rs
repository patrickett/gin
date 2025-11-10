use crate::frontend::prelude::*;
use chumsky::{Parser, input::ValueInput, prelude::*, span::SimpleSpan};

#[derive(Debug, Clone)]
pub struct DocComment {
    content: String,
}

pub fn doc_comment<'t, 's: 't, I>() -> impl Parser<'t, I, DocComment, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    just(Token::DashDashDash)
        .then(
            any()
                .and_is(just(Token::Newline).not())
                .repeated()
                .collect::<Vec<Token>>(),
        )
        .then_ignore(just(Token::Newline))
        .map(|(_dashes, body_tokens)| {
            // PERF: can we get start and end of token slices then just get the
            // full raw slice of the string contents?
            // let content = body_tokens
            //     .into_iter()
            //     .filter_map(|tok| tok.to_string()) // Convert only text tokens
            //     .collect::<Vec<_>>() // Vec<String>
            //     .join(" "); // Join how you prefer

            DocComment {
                content: format!("{:#?}", body_tokens),
            }
        })
}
