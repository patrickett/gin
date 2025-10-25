use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*};

/// `use` can include several different modules seperated by a `,`
///
/// ex.
/// ```gin
/// use http.web, crypto.hash
/// ```
#[derive(Debug, Clone)]
pub struct UseExpr<'src>(Vec<ModuleImport<'src>>);

/// An import is structured like the following:
///
/// `use {module_name}.path.to_sub_mod (import1, ImportTag)`
//  The ending (...) syntax is shared across lambda functions
#[derive(Debug, Clone)]
pub struct ModuleImport<'src> {
    pub path: Path<'src>,
    pub alias: Option<&'src str>,
}

impl<'src> ModuleImport<'src> {
    pub fn new(path: Path<'src>, alias: Option<&'src str>) -> Self {
        Self { path, alias }
    }
}

pub struct FromUse<'src> {
    // from
    path: Path<'src>,
    // use
}

// Use expressions should be at the top of any module.
pub fn import<'t, 's: 't, I>() -> impl Parser<'t, I, UseExpr<'s>, ParserError<'t, 's>>
where
    I: ValueInput<'t, Token = Token<'s>, Span = SimpleSpan>,
{
    let id = select! { Token::Id(name) => name };

    just(Token::Use)
        .ignore_then(
            path()
                .then(just(Token::As).ignore_then(id).or_not())
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .then_ignore(just(Token::Newline)),
        )
        .map(|items| {
            UseExpr(
                items
                    .into_iter()
                    .map(|(path, alias)| ModuleImport { path, alias })
                    .collect::<Vec<ModuleImport>>(),
            )
        })
}
