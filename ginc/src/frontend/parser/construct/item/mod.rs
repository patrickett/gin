use crate::frontend::prelude::*;
mod tag_define;
pub use tag_define::*;

/// This represents a top level item. Not all constructs can be created
/// at the root/top level.
///
/// Unlike languages like Python. This is so you cannot have side effects
#[derive(Debug, Clone)]
pub enum Item {
    Import(UseExpr),
    // definetag and assignment can start with a doc comment
    DefineTag(DefineTag),
    Bind(Bind),
}

pub fn item<'t, 's: 't, I>() -> impl Parser<'t, I, Item, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let expr = expression();
    let tag = tag(expr.clone());
    let comment = comment();

    choice((
        // comment().boxed().map(Item::Comment),
        import().boxed().map(Item::Import),
        define_tag(expression()).map(Item::DefineTag),
        bind(expr.clone(), tag.clone()).map(Item::Bind),
    ))
    .padded_by(just(Token::Newline).repeated()) // ignore newlines around everything
    .padded_by(comment.repeated())
}
