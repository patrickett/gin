use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{
    BundleExportImport, Import, ImportSource, LocalBundleImport, LocalMemberImport, ModPath,
    ModuleImport, Spanned,
};
use span::SpanId;

use crate::cursor::TokenCursor;
use crate::path::parse_id;

/// Dependency or folder segment name in `use` paths (`self` is valid as a flask.jsonc key).
fn parse_import_root(cursor: &mut TokenCursor) -> Option<Intern<String>> {
    match cursor.peek()? {
        Token::SelfInstance => {
            cursor.advance();
            Some(Intern::<String>::from_ref("self"))
        }
        _ => parse_id(cursor),
    }
}

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
            let path_str = *s;
            if path_str.ends_with(".gin") {
                cursor.advance();
                cursor.errors.push(crate::cursor::ParseError {
                    message: "imports target folder modules, not `.gin` files".to_string(),
                    span: span_id,
                });
                return None;
            }
            let path = PathBuf::from(path_str);
            cursor.advance();
            if cursor.eat(&Token::Dot) {
                if cursor.is_at(&Token::ParenOpen) {
                    cursor.advance();
                    let members = parse_bundle_export_list(cursor)?;
                    cursor.expect(&Token::ParenClose)?;
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(span_id, end_span);
                    ImportSource::LocalBundle(LocalBundleImport {
                        root: Intern::<String>::from_ref(""),
                        path_segments: Vec::new(),
                        members,
                        span,
                        local_path: Some(path),
                    })
                } else {
                    let member = parse_single_bundle_member(cursor)?;
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(span_id, end_span);
                    ImportSource::LocalMember(LocalMemberImport {
                        local_path: Some(path),
                        member,
                        span,
                    })
                }
            } else {
                ImportSource::Local(path, span_id)
            }
        }
        Token::Id(_) | Token::SelfInstance => {
            let start_span = cursor.peek_span()?;
            let root = parse_import_root(cursor)?;
            if cursor.eat(&Token::Dot) {
                if cursor.is_at(&Token::ParenOpen) {
                    cursor.advance();
                    let members = parse_bundle_export_list(cursor)?;
                    cursor.expect(&Token::ParenClose)?;
                    let end_span = cursor.last_consumed_span();
                    let span = cursor.merge_span(start_span, end_span);
                    ImportSource::LocalBundle(LocalBundleImport {
                        root,
                        path_segments: Vec::new(),
                        members,
                        span,
                        local_path: None,
                    })
                } else {
                    let mut segments = Vec::new();
                    loop {
                        segments.push(parse_export_name(cursor)?);
                        if !cursor.eat(&Token::Dot) {
                            break;
                        }
                        // Nested bundle: `root.seg1.seg2.(item1, item2)`
                        if cursor.is_at(&Token::ParenOpen) {
                            cursor.advance(); // eat (
                            let members = parse_bundle_export_list(cursor)?;
                            cursor.expect(&Token::ParenClose)?;
                            let end_span = cursor.last_consumed_span();
                            let span = cursor.merge_span(start_span, end_span);
                            return Some((
                                ImportSource::LocalBundle(LocalBundleImport {
                                    root,
                                    path_segments: segments,
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

/// `folder.Symbol` inside `use dep.(folder.Symbol, …)`.
fn parse_bundle_export_name(cursor: &mut TokenCursor) -> Option<Intern<String>> {
    let first = parse_export_name(cursor)?;
    let mut path = first.to_string();
    while cursor.eat(&Token::Dot) {
        let seg = parse_export_name(cursor)?;
        path.push('.');
        path.push_str(seg.as_str());
    }
    Some(Intern::<String>::new(path))
}

fn parse_single_bundle_member(cursor: &mut TokenCursor) -> Option<BundleExportImport> {
    let export_span = cursor.peek_span()?;
    let export = parse_bundle_export_name(cursor)?;
    let alias = if cursor.is_at(&Token::As) {
        cursor.advance();
        Some(parse_export_name(cursor)?)
    } else {
        None
    };
    let end_span = cursor.last_consumed_span();
    let span = cursor.merge_span(export_span, end_span);
    Some(BundleExportImport {
        export,
        alias,
        span,
    })
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
        let export = parse_bundle_export_name(cursor)?;
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
        let output = parse_source_full("use 'utils'.(Foo, Bar)\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        assert_eq!(imports.len(), 1);
        let import = &imports[0].0;
        assert_eq!(import.len(), 1);
        let mi = &import[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.local_path, Some(std::path::PathBuf::from("utils")));
                assert_eq!(lb.members.len(), 2);
                assert_eq!(lb.members[0].export.as_str(), "Foo");
                assert_eq!(lb.members[1].export.as_str(), "Bar");
            }
            other => panic!("expected LocalBundle, got {:?}", other),
        }
    }

    #[test]
    fn parse_local_path_bundle_with_alias() {
        let output = parse_source_full("use 'utils'.(Foo, Bar as Baz)\n");
        let imports: Vec<_> = output.ast.uses().to_vec();
        let mi = &imports[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.local_path, Some(std::path::PathBuf::from("utils")));
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
    fn parse_local_member_import() {
        let output = parse_source_full("use 'utils/math'.add\n");
        let mi = &output.ast.uses()[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalMember(m) => {
                assert_eq!(m.local_path, Some(std::path::PathBuf::from("utils/math")));
                assert_eq!(m.member.export.as_str(), "add");
            }
            other => panic!("expected LocalMember, got {:?}", other),
        }
    }

    #[test]
    fn rejects_gin_file_import_path() {
        let output = parse_source_full("use './a.gin'\n");
        assert!(output.ast.uses().is_empty() || !output.symptoms.is_empty());
    }

    #[test]
    fn parse_package_member_import() {
        let output = parse_source_full("use core.Int\n");
        let mi = &output.ast.uses()[0].0[0];
        match &mi.source {
            ast::ImportSource::Package(mp) => {
                assert_eq!(mp.root.as_str(), "core");
                assert_eq!(mp.segments.len(), 1);
                assert_eq!(mp.segments[0].as_str(), "Int");
            }
            other => panic!("expected Package member import, got {:?}", other),
        }
    }

    #[test]
    fn parse_prefer_member_import_diagnostic() {
        let output = parse_source_full("use core.(Int)\n");
        assert!(output.symptoms.iter().any(|d| {
            matches!(
                &d.code,
                diagnostic::DiagnosticCode::Import(diagnostic::UseSymptom::PreferMemberImport {
                    path_prefix,
                    symbol,
                }) if path_prefix == "core" && symbol == "Int"
            )
        }));
    }

    #[test]
    fn parse_dependency_bundle_with_folder_path() {
        let output = parse_source_full("use self.primitive.(Bool, List)\n");
        let mi = &output.ast.uses()[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.root.as_str(), "self");
                assert_eq!(lb.path_segments.len(), 1);
                assert_eq!(lb.path_segments[0].as_str(), "primitive");
                assert_eq!(lb.members.len(), 2);
                assert_eq!(lb.members[0].export.as_str(), "Bool");
                assert_eq!(lb.members[1].export.as_str(), "List");
            }
            other => panic!("expected LocalBundle, got {:?}", other),
        }
    }

    #[test]
    fn parse_dependency_bundle_dotted_members() {
        let output = parse_source_full("use core.(default.Default, arch.Architecture)\n");
        assert!(
            output.symptoms.is_empty(),
            "parse errors: {:?}",
            output.symptoms
        );
        let mi = &output.ast.uses()[0].0[0];
        match &mi.source {
            ast::ImportSource::LocalBundle(lb) => {
                assert_eq!(lb.root.as_str(), "core");
                assert_eq!(lb.members.len(), 2);
                assert_eq!(lb.members[0].export.as_str(), "default.Default");
                assert_eq!(lb.members[1].export.as_str(), "arch.Architecture");
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
