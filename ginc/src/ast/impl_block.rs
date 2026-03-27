use crate::prelude::*;
use chumsky::span::SimpleSpan;
use std::hash::{Hash, Hasher};

/// A trait implementation block: `Args.Iterator (next: ...)`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplBlock {
    pub type_name: IStr,
    pub type_name_span: SimpleSpan,
    pub trait_name: IStr,
    pub methods: DefMap,
}

impl Hash for ImplBlock {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_name.hash(state);
        self.trait_name.hash(state);
        let mut keys: Vec<_> = self.methods.keys().collect();
        keys.sort();
        for k in keys {
            k.hash(state);
            self.methods[k].hash(state);
        }
    }
}

/// Parse a trait impl block: `Tag.Tag (binds...)`
///
/// Example:
/// ```gin
/// Args.Iterator (
///     next:
///         ...body...
///     return result
/// )
/// ```
pub fn impl_block<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, ImplBlock, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let tag_name = select! { Token::Tag(name) => IStr::new(name.to_string()) };
    let tag_name_with_span = tag_name.map_with(|name, e| (name, e.span()));

    let header = tag_name_with_span.then_ignore(just(Token::Dot)).then(tag_name);

    let body = bind(expr.clone()).padded_by(just(Token::Newline).repeated());

    let methods = just(Token::ParenOpen)
        .ignore_then(just(Token::Newline).repeated())
        .ignore_then(just(Token::Indent).or_not())
        .ignore_then(body.repeated().collect::<Vec<_>>())
        .then_ignore(just(Token::Dedent).or_not())
        .then_ignore(just(Token::Newline).repeated())
        .then_ignore(just(Token::ParenClose));

    header
        .then(methods)
        .map(|(((type_name, type_name_span), trait_name), binds)| {
            let methods = binds.into_iter().map(|b| (b.name(), b)).collect();
            ImplBlock {
                type_name,
                type_name_span,
                trait_name,
                methods,
            }
        })
}
