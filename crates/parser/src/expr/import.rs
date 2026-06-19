use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{
    BundleExportImport, Import, ImportSource, LocalBundleImport, LocalMemberImport, ModPath,
    ModuleImport, Spanned,
};
use span::SpanId;

use crate::cursor::TokenCursor;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Dependency or folder segment name in `use` paths (`self` is valid as a flask.jsonc key).
    fn parse_import_root(&mut self) -> Option<(Intern<String>, SpanId)> {
        match self.peek()? {
            Token::SelfInstance => {
                let span = self.peek_span()?;
                self.advance();
                Some((Intern::<String>::from_ref("self"), span))
            }
            _ => {
                let span = self.peek_span()?;
                let id = self.parse_id()?;
                Some((id, span))
            }
        }
    }

    pub fn parse_import(&mut self) -> Option<Import> {
        self.expect(&Token::Use)?;

        let first = self.parse_import_source()?;
        let mut imports = vec![first];

        while self.eat(&Token::Comma) {
            let next = self.parse_import_source()?;
            imports.push(next);
        }

        Some(Import(
            imports
                .into_iter()
                .map(|(source, alias)| ModuleImport { source, alias })
                .collect(),
        ))
    }

    fn parse_import_source(&mut self) -> Option<(ImportSource, Option<Intern<String>>)> {
        let source = match self.peek()? {
            Token::String(s) => {
                let span_id = self.peek_span()?;
                let path_str = *s;
                if path_str.ends_with(".gin") {
                    self.advance();
                    self.errors.push(crate::cursor::ParseError {
                        message: "imports target folder modules, not `.gin` files".to_string(),
                        span: span_id,
                    });
                    return None;
                }
                let path = PathBuf::from(path_str);
                self.advance();
                if self.eat(&Token::Dot) {
                    if self.is_at(&Token::ParenOpen) {
                        self.advance();
                        let members = self.parse_bundle_export_list()?;
                        self.expect(&Token::ParenClose)?;
                        let end_span = self.last_consumed_span();
                        let span = self.merge_span(span_id, end_span);
                        ImportSource::LocalBundle(LocalBundleImport {
                            root: Intern::<String>::from_ref(""),
                            path_segments: Vec::new(),
                            members,
                            span,
                            local_path: Some(path),
                        })
                    } else {
                        let member = self.parse_single_bundle_member()?;
                        let end_span = self.last_consumed_span();
                        let span = self.merge_span(span_id, end_span);
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
                let start_span = self.peek_span()?;
                let (root_name, root_span) = self.parse_import_root()?;
                if self.eat(&Token::Dot) {
                    if self.is_at(&Token::ParenOpen) {
                        self.advance();
                        let members = self.parse_bundle_export_list()?;
                        self.expect(&Token::ParenClose)?;
                        let end_span = self.last_consumed_span();
                        let span = self.merge_span(start_span, end_span);
                        ImportSource::LocalBundle(LocalBundleImport {
                            root: root_name,
                            path_segments: Vec::new(),
                            members,
                            span,
                            local_path: None,
                        })
                    } else {
                        let mut segments = Vec::new();
                        let mut segment_spans = Vec::new();
                        loop {
                            let seg_span = self.peek_span()?;
                            segments.push(self.parse_export_name()?);
                            segment_spans.push(seg_span);
                            if !self.eat(&Token::Dot) {
                                break;
                            }
                            // Nested bundle: `root.seg1.seg2.(item1, item2)`
                            if self.is_at(&Token::ParenOpen) {
                                self.advance(); // eat (
                                let members = self.parse_bundle_export_list()?;
                                self.expect(&Token::ParenClose)?;
                                let end_span = self.last_consumed_span();
                                let span = self.merge_span(start_span, end_span);
                                return Some((
                                    ImportSource::LocalBundle(LocalBundleImport {
                                        root: root_name,
                                        path_segments: segments,
                                        members,
                                        span,
                                        local_path: None,
                                    }),
                                    None,
                                ));
                            }
                        }
                        let end_span = self.last_consumed_span();
                        let span = self.merge_span(start_span, end_span);
                        ImportSource::Package(Spanned::new(
                            ModPath::new_with_spans(root_name, root_span, segments, segment_spans),
                            span,
                        ))
                    }
                } else {
                    ImportSource::Package(Spanned::new(
                        ModPath::new_with_spans(root_name, root_span, Vec::new(), Vec::new()),
                        start_span,
                    ))
                }
            }
            Token::Tag(name) => {
                let export_span = self.peek_span()?;
                let export = self.intern(name);
                self.advance();
                // Check for `as alias`
                let alias = if self.is_at(&Token::As) {
                    self.advance();
                    Some(self.parse_export_name()?)
                } else {
                    None
                };
                let end_span = self.last_consumed_span();
                let span = self.merge_span(export_span, end_span);
                ImportSource::CurrentModule {
                    member: BundleExportImport {
                        export,
                        alias,
                        span,
                    },
                }
            }
            _ => {
                let span = self.peek_span().unwrap_or(SpanId::INVALID);
                self.errors.push(crate::cursor::ParseError {
                    message: "expected string path or module path after `use`".to_string(),
                    span,
                });
                return None;
            }
        };

        let alias = if self.is_at(&Token::As) {
            self.advance();
            self.parse_id()
        } else {
            None
        };

        Some((source, alias))
    }

    /// Parse an export name inside a bundle import `dep.(...)`.
    /// Accepts both `Id` and `Tag` tokens since exported symbols can start with
    /// either lowercase or uppercase letters.
    fn parse_export_name(&mut self) -> Option<Intern<String>> {
        match self.peek()? {
            Token::Id(name) | Token::Tag(name) => {
                let id = self.intern(name);
                self.advance();
                Some(id)
            }
            _ => None,
        }
    }

    /// `folder.Symbol` inside `use dep.(folder.Symbol, …)`.
    fn parse_bundle_export_name(&mut self) -> Option<Intern<String>> {
        let first = self.parse_export_name()?;
        let mut path = first.to_string();
        while self.eat(&Token::Dot) {
            let seg = self.parse_export_name()?;
            path.push('.');
            path.push_str(seg.as_str());
        }
        Some(Intern::<String>::new(path))
    }

    fn parse_single_bundle_member(&mut self) -> Option<BundleExportImport> {
        let export_span = self.peek_span()?;
        let export = self.parse_bundle_export_name()?;
        let alias = if self.is_at(&Token::As) {
            self.advance();
            Some(self.parse_export_name()?)
        } else {
            None
        };
        let end_span = self.last_consumed_span();
        let span = self.merge_span(export_span, end_span);
        Some(BundleExportImport {
            export,
            alias,
            span,
        })
    }

    fn parse_bundle_export_list(&mut self) -> Option<Vec<BundleExportImport>> {
        let mut members = Vec::new();

        self.skip_layout();

        if self.is_at(&Token::ParenClose) {
            let span = self.peek_span().unwrap_or(SpanId::INVALID);
            self.errors.push(crate::cursor::ParseError {
                message: "expected at least one export inside `.(...)`".to_string(),
                span,
            });
            return Some(members);
        }

        loop {
            let export_span = self.peek_span()?;
            let export = self.parse_bundle_export_name()?;
            let alias = if self.is_at(&Token::As) {
                self.advance();
                Some(self.parse_export_name()?)
            } else {
                None
            };
            let end_span = self.last_consumed_span();
            let span = self.merge_span(export_span, end_span);
            members.push(BundleExportImport {
                export,
                alias,
                span,
            });

            if self.is_at(&Token::ParenClose) {
                break;
            }

            if self.eat_list_separator() {
                continue;
            }

            self.error(
                "expected ',' or newline between exports",
                self.current_span(),
            );
            break;
        }

        self.skip_layout();
        Some(members)
    }
}

#[cfg(test)]
mod tests {
    use crate::query::SourceParseExt;

    #[test]
    fn parse_local_path_bundle_import() {
        let output = "use 'utils'.(Foo, Bar)\n".parse_source_full();
        let imports: Vec<_> = output.ast.uses.to_vec();
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
        let output = "use 'utils'.(Foo, Bar as Baz)\n".parse_source_full();
        let imports: Vec<_> = output.ast.uses.to_vec();
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
        let output = "use 'utils/math'.add\n".parse_source_full();
        let mi = &output.ast.uses[0].0[0];
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
        let output = "use './a.gin'\n".parse_source_full();
        assert!(output.ast.uses.is_empty() || !output.symptoms.is_empty());
    }

    #[test]
    fn parse_package_member_import() {
        let output = "use core.Int\n".parse_source_full();
        let mi = &output.ast.uses[0].0[0];
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
        let output = "use core.(Int)\n".parse_source_full();
        assert!(output.symptoms.iter().any(|d| {
            d.code.slug() == "use-prefer-member-import"
                && d.message == "prefer use core.Int over a single-item .(...) bundle"
        }));
    }

    #[test]
    fn parse_dependency_bundle_with_folder_path() {
        let output = "use self.primitive.(Bool, List)\n".parse_source_full();
        let mi = &output.ast.uses[0].0[0];
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
        let output = "use core.(default.Default, arch.Architecture)\n".parse_source_full();
        assert!(
            output.symptoms.is_empty(),
            "parse errors: {:?}",
            output.symptoms
        );
        let mi = &output.ast.uses[0].0[0];
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
        let output = "use core.(Int, Byte)\n".parse_source_full();
        let imports: Vec<_> = output.ast.uses.to_vec();
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
        let output = "use Str, Int, Byte\n".parse_source_full();
        let imports: Vec<_> = output.ast.uses.to_vec();
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
        let output = "use Str as string, Int, Byte\n".parse_source_full();
        let imports: Vec<_> = output.ast.uses.to_vec();
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
