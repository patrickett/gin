use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{
    BundleExportImport, Import, ImportSource, LocalBundleImport, ModPath, ModuleImport, Spanned,
};
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
            // Check for `'path'.(items)`
            if cursor.eat(&Token::Dot) && cursor.is_at(&Token::ParenOpen) {
                cursor.advance(); // eat (
                let members = parse_bundle_export_list(cursor)?;
                cursor.expect(&Token::ParenClose)?;
                let end_span = cursor.last_consumed_span();
                let span = cursor.merge_span(span_id, end_span);
                ImportSource::LocalBundle(LocalBundleImport {
                    root: Intern::<String>::from_ref(""),
                    members,
                    span,
                    local_path: Some(path),
                })
            } else {
                ImportSource::Local(path, span_id)
            }
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
                        local_path: None,
                    })
                } else {
                    let mut segments = Vec::new();
                    loop {
                        segments.push(parse_id(cursor)?);
                        if !cursor.eat(&Token::Dot) {
                            break;
                        }
                        // Nested bundle: `root.seg1.seg2.(item1, item2)`
                        if cursor.is_at(&Token::ParenOpen) {
                            // Combine root and segments into the bundle root via a dotted qualifier,
                            // and treat the remaining content as bundle members.
                            let mut bundle_root = root.to_string();
                            for seg in &segments {
                                bundle_root.push('.');
                                bundle_root.push_str(seg.as_str());
                            }
                            let bundle_root = Intern::<String>::new(bundle_root);
                            cursor.advance(); // eat (
                            let members = parse_bundle_export_list(cursor)?;
                            cursor.expect(&Token::ParenClose)?;
                            let end_span = cursor.last_consumed_span();
                            let span = cursor.merge_span(start_span, end_span);
                            return Some((
                                ImportSource::LocalBundle(LocalBundleImport {
                                    root: bundle_root,
                                    members,
                                    span,
                                    local_path: None,
                                }),
                                None,
                            ));
                        }
                    }
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(start_span, end_span);
                    ImportSource::Package(Spanned::new(ModPath { root, segments }, span))
                }
            } else {
                ImportSource::Package(Spanned::new(
                    ModPath {
                        root,
                        segments: Vec::new(),
                    },
                    start_span,
                ))
            }
        }
        Token::Tag(name) => {
            let export_span = cursor.peek_span()?;
            let export = cursor.intern(name);
            cursor.advance();
            // Check for `as alias`
            let alias = if cursor.is_at(&Token::As) {
                cursor.advance();
                Some(parse_export_name(cursor)?)
            } else {
                None
            };
            let end_span = cursor.last_consumed_span();
            let span = cursor.merge_span(export_span, end_span);
            ImportSource::CurrentModule {
                member: BundleExportImport {
                    export,
                    alias,
                    span,
                },
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

/// Parse an export name inside a bundle import `dep.(...)`.
/// Accepts both `Id` and `Tag` tokens since exported symbols can start with
/// either lowercase or uppercase letters.
fn parse_export_name(cursor: &mut TokenCursor) -> Option<Intern<String>> {
    match cursor.peek()? {
        Token::Id(name) | Token::Tag(name) => {
            let id = cursor.intern(name);
            cursor.advance();
            Some(id)
        }
        _ => None,
    }
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
        let export_span = cursor.peek_span()?;
        let export = parse_export_name(cursor)?;
        let alias = if cursor.is_at(&Token::As) {
            cursor.advance();
            Some(parse_export_name(cursor)?)
        } else {
            None
        };
        let end_span = cursor.last_consumed_span();
        let span = cursor.merge_span(export_span, end_span);
        members.push(BundleExportImport {
            export,
            alias,
            span,
        });

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

#[cfg(test)]
mod tests {
    use crate::query::parse_source_full;

    #[test]
    fn parse_local_path_bundle_import() {
        let output = parse_source_full("use './a.gin'.(Foo, Bar)\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        assert_eq!(imports.len(), 1);
        let import = &imports[0].0;
        assert_eq!(import.len(), 1);
        let mi = &import[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.local_path, Some(std::path::PathBuf::from("./a.gin")));
                assert_eq!(lb.members.len(), 2);
                assert_eq!(lb.members[0].export.as_str(), "Foo");
                assert_eq!(lb.members[1].export.as_str(), "Bar");
            }
            other => panic!("expected LocalBundle, got {:?}", other),
        }
    }

    #[test]
    fn parse_local_path_bundle_with_alias() {
        let output = parse_source_full("use './a.gin'.(Foo, Bar as Baz)\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        let mi = &imports[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.local_path, Some(std::path::PathBuf::from("./a.gin")));
                assert_eq!(lb.members.len(), 2);
                assert_eq!(lb.members[0].export.as_str(), "Foo");
                assert!(lb.members[0].alias.is_none());
                assert_eq!(lb.members[1].export.as_str(), "Bar");
                assert_eq!(lb.members[1].alias.unwrap().as_str(), "Baz");
            }
            other => panic!("expected LocalBundle, got {:?}", other),
        }
    }

    #[test]
    fn parse_dependency_bundle_import() {
        let output = parse_source_full("use core.(Int, Byte)\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        let mi = &imports[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.root.as_str(), "core");
                assert!(lb.local_path.is_none());
                assert_eq!(lb.members.len(), 2);
            }
            other => panic!("expected LocalBundle, got {:?}", other),
        }
    }

    #[test]
    fn parse_current_module_tag_import() {
        let output = parse_source_full("use Str, Int, Byte\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        assert_eq!(imports.len(), 1);
        let import = &imports[0].0;
        assert_eq!(import.len(), 3);

        // First: Str
        match &import[0].source {
            ast::ImportSource::CurrentModule { member } => {
                assert_eq!(member.export.as_str(), "Str");
                assert!(member.alias.is_none());
            }
            other => panic!("expected CurrentModule, got {:?}", other),
        }

        // Second: Int
        match &import[1].source {
            ast::ImportSource::CurrentModule { member } => {
                assert_eq!(member.export.as_str(), "Int");
            }
            other => panic!("expected CurrentModule, got {:?}", other),
        }

        // Third: Byte
        match &import[2].source {
            ast::ImportSource::CurrentModule { member } => {
                assert_eq!(member.export.as_str(), "Byte");
            }
            other => panic!("expected CurrentModule, got {:?}", other),
        }
    }

    #[test]
    fn parse_current_module_tag_import_with_alias() {
        let output = parse_source_full("use Str as string, Int, Byte\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        let mi = &imports[0].0[0];
        match &mi.source {
            ast::ImportSource::CurrentModule { member } => {
                assert_eq!(member.export.as_str(), "Str");
                assert_eq!(member.alias.unwrap().as_str(), "string");
            }
            other => panic!("expected CurrentModule, got {:?}", other),
        }
    }
}
