use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{BundleExportImport, Import, ImportSource, LocalBundleImport, ModPath, ModuleImport};
use span::SpanId;

use crate::cursor::TokenCursor;
use crate::path::parse_id;

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
        Token::Id(_) => {
            let start_span = cursor.peek_span()?;
            let root = parse_id(cursor)?;
            if cursor.eat(&Token::Dot) {
                if cursor.is_at(&Token::ParenOpen) {
                    cursor.advance();
                    let members = parse_bundle_export_list(cursor)?;
                    cursor.expect(&Token::ParenClose)?;
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(start_span, end_span);
                    ImportSource::LocalBundle(LocalBundleImport {
                        root,
                        members,
                        span,
                    })
                } else {
                    let mut segments = Vec::new();
                    loop {
                        segments.push(parse_id(cursor)?);
                        if !cursor.eat(&Token::Dot) {
                            break;
                        }
                    }
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(start_span, end_span);
                    ImportSource::Package(ModPath {
                        root,
                        segments,
                        span,
                    })
                }
            } else {
                ImportSource::Package(ModPath {
                    root,
                    segments: Vec::new(),
                    span: start_span,
                })
            }
        }
        _ => {
            let span = cursor.peek_span().unwrap_or(SpanId::INVALID);
            cursor.errors.push(crate::cursor::ParseError {
                message: "expected string path or module path after `use`".to_string(),
                span,
            });
            return None;
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

fn parse_bundle_export_list(cursor: &mut TokenCursor) -> Option<Vec<BundleExportImport>> {
    let mut members = Vec::new();

    if cursor.is_at(&Token::ParenClose) {
        let span = cursor.peek_span().unwrap_or(SpanId::INVALID);
        cursor.errors.push(crate::cursor::ParseError {
            message: "expected at least one export inside `.(...)`".to_string(),
            span,
        });
        return Some(members);
    }

    loop {
        let export = parse_id(cursor)?;
        let alias = if cursor.is_at(&Token::As) {
            cursor.advance();
            Some(parse_id(cursor)?)
        } else {
            None
        };
        members.push(BundleExportImport { export, alias });

        if cursor.eat(&Token::Comma) {
            if cursor.is_at(&Token::ParenClose) {
                break;
            }
            continue;
        }
        break;
    }

    Some(members)
}
