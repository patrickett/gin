use crate::frontend::prelude::*;

pub fn range<'t, 's: 't, I>()
-> impl Parser<'t, I, std::ops::Range<i64>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let int = select! { Token::Int(int) => int };

    int.then_ignore(just(Token::DotDot))
        .then(int)
        .map(|(start, end)| std::ops::Range { start, end })
}
