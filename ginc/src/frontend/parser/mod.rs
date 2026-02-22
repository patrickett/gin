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
    let item_parser = item();

    import()
        .separated_by(just(Newline))
        .collect::<Vec<_>>()
        .then(item_parser.clone().repeated().collect::<Vec<Item>>())
        .then(
            just(Private)
                .padded_by(just(Newline).repeated())
                .ignore_then(item_parser.repeated().collect::<Vec<Item>>())
                .or_not(),
        )
        .map(|((imports, public_items), private_items)| {
            let mut tags = TagMap::new();
            let mut defs = DefMap::new();
            let mut private_defs = std::collections::HashSet::new();
            let mut private_tags = std::collections::HashSet::new();

            for item in public_items {
                collect_item(item, &mut tags, &mut defs);
            }

            if let Some(priv_items) = private_items {
                for item in priv_items {
                    match &item.value {
                        ItemValue::TagValue(name, _) => {
                            private_tags.insert(name.clone());
                        }
                        ItemValue::DefValue(name, _) => {
                            private_defs.insert(name.clone());
                        }
                    }
                    collect_item(item, &mut tags, &mut defs);
                }
            }

            FileAst {
                uses: imports,
                tags,
                defs,
                private_defs,
                private_tags,
            }
        })
}

fn collect_item(item: Item, tags: &mut TagMap, defs: &mut DefMap) {
    match item.value {
        ItemValue::TagValue(name, bind) => {
            tags.insert(
                name,
                Documented {
                    item: bind,
                    doc: item.doc_comment,
                },
            );
        }
        ItemValue::DefValue(name, bind) => {
            defs.insert(
                name,
                Documented {
                    item: bind,
                    doc: item.doc_comment,
                },
            );
        }
    }
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
