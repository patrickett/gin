use crate::prelude::*;
use chumsky::{input::ValueInput, prelude::*, span::SimpleSpan};
use std::path::PathBuf;

/// `use` can include several different modules seperated by a `,`
///
/// ex.
/// ```gin
/// use http.web, crypto.hash
/// use './math' as math
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Import(pub Vec<ModuleImport>);

// TODO: for scripts we want to support git urls as if they were in flask.json
// but in use statements so scripts can use remote depenecies
// `use 'https://github.com/gin/db_project.git' as db`

// TODO: Implement import wildcard support (*)
// `use core.http (...)`

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportSource {
    /// Top level name defined in `flask.json` ex. `use http.*`
    Package(ModPath),
    /// Path to a module on disk ex. `use '../http' as http`
    Local(PathBuf, SimpleSpan),
}

/// An import is structured like the following:
///
/// `use {module_name}.path.to_sub_mod (import1, ImportTag)`
/// `use './local/folder' as alias`
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleImport {
    pub source: ImportSource,
    pub alias: Option<Intern::<::std::string::String>>,
}

impl ModuleImport {
    /// Compute the default alias name from the import source.
    ///
    /// - `Package(path)` → last segment, or root if no segments
    /// - `Local(path)` → last component of the folder path
    pub fn effective_name(&self) -> String {
        match &self.source {
            ImportSource::Package(path) => path
                .segments
                .last()
                .map(|s| s.to_string())
                .unwrap_or_else(|| path.root.to_string()),
            ImportSource::Local(path, _) => path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string(),
        }
    }
}

// Use expressions should be at the top of any module.
pub fn import<'t, I>() -> impl Parser<'t, I, Import, ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let id = id_token();

    let source = choice((
        select! { Token::String(s) => PathBuf::from(s) }
            .map_with(|path, e| ImportSource::Local(path, e.span())),
        path().map(ImportSource::Package),
    ));

    just(Token::Use)
        .ignore_then(
            source
                .then(just(Token::As).ignore_then(id).or_not())
                .separated_by(just(Token::Comma))
                .collect::<Vec<_>>()
                .then_ignore(just(Token::Newline).or_not()),
        )
        .map(|items: Vec<_>| {
            Import(
                items
                    .into_iter()
                    .map(|(source, alias)| ModuleImport { source, alias })
                    .collect::<Vec<ModuleImport>>(),
            )
        })
}
