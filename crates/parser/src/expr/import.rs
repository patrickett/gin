use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{Import, ImportSource, ModuleImport};

use crate::cursor::TokenCursor;
use crate::path::{parse_id, parse_path};

pub fn parse_import(cursor: &mut TokenCursor) -> Option<Import> {
    cursor.expect(&Token::Use)?;

    let first = parse_import_source(cursor)?;
    let mut imports = vec![first];

    while cursor.eat(&Token::Comma) {
        let next = parse_import_source(cursor)?;
        imports.push(next);
    }

    Some(Import(
        imports
            .into_iter()
            .map(|(source, alias)| ModuleImport { source, alias })
            .collect(),
    ))
}

fn parse_import_source(cursor: &mut TokenCursor) -> Option<(ImportSource, Option<Intern<String>>)> {
    let source = match cursor.peek()? {
        Token::String(s) => {
            let span_id = cursor.peek_span()?;
            let path = PathBuf::from(*s);
            cursor.advance();
            ImportSource::Local(path, span_id)
        }
        _ => {
            let path = parse_path(cursor)?;
            ImportSource::Package(path)
        }
    };

    let alias = if cursor.is_at(&Token::As) {
        cursor.advance();
        parse_id(cursor)
    } else {
        None
    };

    Some((source, alias))
}
