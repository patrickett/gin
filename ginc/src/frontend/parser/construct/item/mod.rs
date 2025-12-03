use crate::frontend::prelude::*;
mod tag_value;
pub use tag_value::*;
mod def_value;
pub use def_value::*;

/// This represents a top level item. Not all constructs can be created
/// at the root/top level.
///
/// Unlike languages like Python. This is so you cannot have side effects
#[derive(Debug, Clone)]
pub struct Item {
    pub doc_comment: Option<DocComment>,
    pub value: ItemValue,
}

#[derive(Debug, Clone)]
pub enum ItemValue {
    TagValue(TagName, Params<TagValue>),
    DefValue(DefName, Params<DefValue>),
}

pub fn item<'t, 's: 't, I>() -> impl Parser<'t, I, Item, ParserError<'t, 's>> + Clone
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let expr = expression();
    doc_comment()
        .or_not()
        .then(
            bind(expr)
                .padded_by(just(Token::Newline).repeated()) // ignore newlines around everything
                .padded_by(comments()),
        )
        .map(|(doc_comment, bind)| Item {
            doc_comment,
            value: match bind {
                Bind::Tag(tag_name, bind) => ItemValue::TagValue(tag_name, bind),
                Bind::Def(def_name, bind) => ItemValue::DefValue(def_name, bind),
            },
        })
}
