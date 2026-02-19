//! Tokenize query helper - wraps the existing logos lexer.
//!
//! This is not a tracked query since tokens are cheap to regenerate
//! and we only need to cache the final AST.

use crate::database::File;
use crate::database::input_database::Db;
use crate::frontend::lexer::GinLexer;
use crate::frontend::Token;
use chumsky::span::SimpleSpan;

/// Tokenize a Gin source file using the logos lexer.
pub fn tokenize<'db>(db: &'db dyn Db, file: File) -> Vec<(Token<'db>, SimpleSpan)> {
    let contents = file.contents(db);
    GinLexer::new(contents).collect()
}
