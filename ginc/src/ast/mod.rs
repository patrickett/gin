mod expr;
pub use expr::*;
mod ast;
pub use ast::*;
mod parameter;
pub use parameter::*;
mod path;
pub use path::*;
mod tag;
pub use tag::*;
mod doc_comment;
pub use doc_comment::*;
mod declare;
pub use declare::*;
mod pattern;
pub use pattern::*;
mod ident;
pub use ident::*;

use crate::prelude::*;
use chumsky::{input::ValueInput, span::SimpleSpan};

/// Parses a stream of tokens into a `FileAst`.
pub fn token_parser<'t, I>() -> impl Parser<'t, I, FileAst, crate::parse::ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let expr_parser = expression();

    let method_parser = tag(expr_parser.clone())
        .then_ignore(just(Dot))
        .then(bind(expr_parser.clone()))
        .map(|(receiver_type, bind)| bind.with_receiver_type(Some(receiver_type)));

    let element = choice((
        method_parser.map(TopLevelValue::Bind),
        bind(expr_parser.clone()).map(TopLevelValue::Bind),
        declare(expr_parser.clone()).map(TopLevelValue::Tag),
        expr_parser.map(TopLevelValue::Expr),
    ))
    .padded_by(just(Newline).repeated());

    import()
        .separated_by(just(Newline))
        .collect::<Vec<_>>()
        .then(element.clone().repeated().collect::<Vec<_>>())
        .then(
            just(Private)
                .padded_by(just(Newline).repeated())
                .ignore_then(element.repeated().collect::<Vec<_>>())
                .or_not(),
        )
        .map(|((imports, public_elements), private_elements)| {
            let mut tags = TagMap::new();
            let mut defs = DefMap::new();
            let mut private_defs = std::collections::HashSet::new();
            let mut private_tags = std::collections::HashSet::new();
            let mut exprs = Vec::new();

            for el in public_elements {
                collect_top_level(el, &mut tags, &mut defs, &mut exprs);
            }

            if let Some(priv_elements) = private_elements {
                for el in priv_elements {
                    match &el {
                        TopLevelValue::Tag(decl) => {
                            private_tags.insert(decl.name());
                        }
                        TopLevelValue::Bind(bind) => {
                            private_defs.insert(bind.name());
                        }
                        TopLevelValue::Expr(_) => {}
                    }
                    collect_top_level(el, &mut tags, &mut defs, &mut exprs);
                }
            }

            FileAst {
                uses: imports,
                tags,
                defs,
                private_defs,
                private_tags,
                exprs,
            }
        })
}

enum TopLevelValue {
    Tag(Declare),
    Bind(Bind),
    Expr(Expr),
}

fn collect_top_level(
    el: TopLevelValue,
    tags: &mut TagMap,
    defs: &mut DefMap,
    exprs: &mut Vec<Expr>,
) {
    match el {
        TopLevelValue::Tag(decl) => {
            let name = decl.name();
            tags.insert(name, decl);
        }
        TopLevelValue::Bind(bind) => {
            let name = bind.name();
            defs.insert(name, bind);
        }
        TopLevelValue::Expr(expr) => {
            exprs.push(expr);
        }
    }
}
