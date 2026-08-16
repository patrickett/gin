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
                    self.errors
                        .push(crate::cursor::ParseError::invalid_import_target(
                            "imports target folder modules, not `.gin` files",
                            span_id,
                        ));
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
                self.errors
                    .push(crate::cursor::ParseError::expected_import_source(
                        "expected string path or module path after `use`",
                        span,
                    ));
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
            self.errors
                .push(crate::cursor::ParseError::empty_import_bundle(
                    "expected at least one export inside `.(...)`",
                    span,
                ));
            return Some(members);
        }

        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }

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
                if self.is_at(&Token::ParenClose) {
                    break;
                }
                continue;
            }

            self.error(
                "parse-expected-export-separator",
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
#[path = "../../tests/import_tests.rs"]
mod tests;
