use crate::frontend::prelude::*;
mod tag_define;
pub use tag_define::*;

/// This represents a top level item. Not all constructs can be created
/// at the root/top level.
///
/// Unlike languages like Python. This is so you cannot have side effects
#[derive(Debug, Clone)]
pub enum Item<'src> {
    Import(UseExpr<'src>),
    DefineTag(DefineTag<'src>),
    DefineFn(Assignment<'src>),
}

pub fn item<'t, 's: 't, I>() -> impl Parser<'t, I, Item<'s>, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let expr = expression();
    let tag = tag(expr.clone());

    choice((
        import().boxed().map(Item::Import),
        define_tag(expression()).map(Item::DefineTag),
        assignment(expr.clone(), tag.clone()).map(Item::DefineFn),
    ))
    .padded_by(just(Token::Newline).repeated()) // ignore newlines around everything
}
