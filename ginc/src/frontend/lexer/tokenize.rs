// TODO: stop ignoring doc comments, make attribute on item

use crate::database::File;
use crate::database::input_database::Db;
use crate::frontend::lexer::GinLexer;
use crate::frontend::Token;
use chumsky::span::SimpleSpan;

#[salsa::tracked]
pub fn tokenize<'db>(db: &'db dyn Db, file: File) -> Vec<(Token, SimpleSpan)> {
    let contents = file.contents(db);
    GinLexer::new(contents)
        .filter(|(t, _)| !matches!(t, Token::Comment(_) | Token::DocComment(_)))
        .collect()
}
