use crate::frontend::prelude::*;
use chumsky::{input::ValueInput, prelude::*};

/// `use` can include several different modules seperated by a `,`
///
/// ex.
/// ```gin
/// use http.web, crypto.hash
/// ```
#[derive(Debug, Clone)]
pub struct Import(pub Vec<ModuleImport>);

// TODO: import * from module
// `use core.http (...)`

/// An import is structured like the following:
///
/// `use {module_name}.path.to_sub_mod (import1, ImportTag)`
//  The ending (...) syntax is shared across lambda functions
#[derive(Debug, Clone)]
pub struct ModuleImport {
    pub path: Path,
    pub alias: Option<String>,
}

impl ModuleImport {
    pub fn new(path: Path, alias: Option<String>) -> Self {
        Self { path, alias }
    }
}

// Use expressions should be at the top of any module.
pub fn import<'t, 's: 't, I>() -> impl Parser<'t, I, Import, ParserError<'t, 's>>
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
            Import(
                items
                    .into_iter()
                    .map(|(path, alias)| ModuleImport {
                        path,
                        alias: alias.map(|a| a.to_string()),
                    })
                    .collect::<Vec<ModuleImport>>(),
            )
        })
}
