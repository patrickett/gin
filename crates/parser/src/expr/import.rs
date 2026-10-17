use diagnostic::DiagnosticCode;
use diagnostic::code::{ExpectedSyntaxCode, ParseCode, SyntaxCode};
use std::path::PathBuf;

use internment::Intern;
use lexer::Token;

use ast::{
    BundleExportImport, HasSpanId, Import, ImportSource, LocalBundleImport, ModPath, ModuleImport,
    Spanned,
};
use span::SpanId;

use crate::cursor::TokenCursor;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Dependency or folder segment name in `use` paths (`self` is valid as a mod.flask key).
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
        let mut source = match self.peek()? {
            Token::String(s) => {
                let span_id = self.peek_span()?;
                let path_str = *s;
                if path_str.ends_with(".gin") {
                    self.advance();
                    if self.eat(&Token::As) {
                        let alias = self.parse_id()?;
                        if self.is_at(&Token::Dot) {
                            self.errors.push(crate::cursor::ParseError::invalid_import_target(
                                "`.gin` file imports use the namespace alias; member syntax is not allowed",
                                span_id));
                            return None;
                        }
                        return Some((
                            ImportSource::LocalFile(PathBuf::from(path_str), span_id),
                            Some(alias),
                        ));
                    }
                    self.errors
                        .push(crate::cursor::ParseError::invalid_import_target(
                            "`.gin` file imports require an alias (use `as Alias`)",
                            span_id,
                        ));
                    return None;
                }
                let path = PathBuf::from(path_str);
                self.advance();
                if self.is_at(&Token::Dot) {
                    // parse-legacy-import-selection
                    self.error_here(
                        DiagnosticCode::Parse(ParseCode::Syntax(SyntaxCode::LegacyImportSelection)),
                        "select exports after a space: use 'module' name, other",
                    );
                    return None;
                }
                ImportSource::Local(path, span_id)
            }

            Token::Id(_) | Token::SelfInstance => {
                let start_span = self.peek_span()?;
                let (root_name, root_span) = self.parse_import_root()?;
                let mut segments = Vec::new();
                let mut segment_spans = Vec::new();
                while self.eat(&Token::Dot) {
                    if self.is_at(&Token::ParenOpen) {
                        // parse-legacy-import-selection
                        self.error_here(
                            DiagnosticCode::Parse(ParseCode::Syntax(
                                SyntaxCode::LegacyImportSelection,
                            )),
                            "select exports after a space: use module name, other",
                        );
                        return None;
                    }
                    segment_spans.push(self.peek_span()?);
                    segments.push(self.parse_export_name()?);
                }
                let span = self.merge_span(start_span, self.last_consumed_span());
                ImportSource::Package(Spanned::new(
                    ModPath::new_with_spans(root_name, root_span, segments, segment_spans),
                    span,
                ))
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

        if matches!(source, ImportSource::Local(..) | ImportSource::Package(..))
            && (self.is_at(&Token::Indent)
                || matches!(self.peek_at(0), Some(Token::Id(_) | Token::Tag(_))))
        {
            let start = source.span_id();
            let members = self.parse_import_members();
            let span = self.merge_span(start, self.last_consumed_span());
            source = match source {
                ImportSource::Local(path, _) => ImportSource::LocalBundle(LocalBundleImport {
                    root: Intern::from_ref(""),
                    path_segments: Vec::new(),
                    module_path: None,
                    members,
                    span,
                    local_path: Some(path),
                }),
                ImportSource::Package(path) => ImportSource::LocalBundle(LocalBundleImport {
                    root: path.root,
                    path_segments: path.segments.clone(),
                    module_path: Some(Box::new(path.clone())),
                    members,
                    span,
                    local_path: None,
                }),
                _ => unreachable!(),
            };
        }

        let alias = if self.is_at(&Token::As) {
            self.advance();
            self.parse_id()
        } else {
            None
        };

        Some((source, alias))
    }

    fn parse_import_members(&mut self) -> Vec<BundleExportImport> {
        let block = self.eat(&Token::Indent);
        let mut members = Vec::new();
        loop {
            if block && self.eat(&Token::Dedent) {
                break;
            }
            let Some(member) = self.parse_single_bundle_member() else {
                // parse-expected-import-export
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ImportExport)),
                    "expected an export name after the module",
                );
                break;
            };
            members.push(member);
            if matches!(self.peek_at(0), Some(Token::Comma)) {
                self.advance_raw();
                continue;
            }
            if block && matches!(self.peek_at(0), Some(Token::Newline | Token::Dedent)) {
                continue;
            }
            if matches!(self.peek_at(0), Some(Token::Id(_) | Token::Tag(_))) {
                // parse-expected-export-separator
                self.error_here(
                    DiagnosticCode::Parse(ParseCode::Expected(ExpectedSyntaxCode::ExportSeparator)),
                    "expected ',' or newline between exports",
                );
            }
            break;
        }
        members
    }

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

    /// `folder.Symbol` inside `use dep folder.Symbol, …`.
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
}
#[cfg(test)]
#[path = "../../tests/import_tests.rs"]
mod tests;
