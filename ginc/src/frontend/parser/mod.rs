pub mod construct;

pub mod parse;

use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, span::SimpleSpan};

pub use parse::*;
pub type Spanned<T> = (T, SimpleSpan);
pub type ParserError<'t> = extra::Err<Rich<'t, Token<'t>>>;

/// Parses a stream of tokens
pub fn token_parser<'t, I>() -> impl Parser<'t, I, FileAst, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;
    import()
        .padded_by(comments().or_not())
        .separated_by(just(Newline))
        .collect::<Vec<_>>()
        .then(
            item()
                .padded_by(comments().or_not())
                .repeated()
                .collect::<(TagMap, DefMap)>(),
        )
        .map(|(imports, (tags, defs))| FileAst {
            uses: imports,
            tags,
            defs,
        })
}

impl FromIterator<Item> for (TagMap, DefMap) {
    fn from_iter<I: IntoIterator<Item = Item>>(iter: I) -> Self {
        let mut tags = TagMap::new();
        let mut defs = DefMap::new();

        for item in iter {
            match item.value {
                ItemValue::TagValue(name, bind) => {
                    let doc = Documented {
                        item: bind,
                        doc: item.doc_comment,
                    };

                    tags.insert(name, doc);
                }
                ItemValue::DefValue(name, bind) => {
                    let doc = Documented {
                        item: bind,
                        doc: item.doc_comment,
                    };

                    defs.insert(name, doc);
                }
            }
        }

        (tags, defs)
    }
}
