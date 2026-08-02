use crate::Backend;
use crate::diagnostics::DiagnosticConverter;

use resolve::{CursorDefinition, DefLocation, ImportNavLocation};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        let file_path = match Backend::file_path_from_uri(&uri) {
            Some(p) => p,
            None => return Ok(None),
        };

        let locations = self
            .run_blocking_request("goto_definition", move |this| {
                let snapshot = this.snapshot();
                let doc = snapshot.cache.document_snapshot(&file_path)?;
                let byte_pos = doc.line_index.position_to_byte(
                    &doc.source,
                    position.line,
                    position.character,
                )?;
                let def = snapshot
                    .cache
                    .goto_definition(&file_path, byte_pos as u32)?;
                Self::cursor_definition_to_response(&uri, &doc.source, &doc.line_index, def)
            })
            .await;

        Ok(locations.flatten())
    }

    fn cursor_definition_to_response(
        uri: &Url,
        source: &str,
        line_index: &ast::source::LineIndex,
        def: CursorDefinition,
    ) -> Option<GotoDefinitionResponse> {
        match def {
            CursorDefinition::SameFile(range) => Some(GotoDefinitionResponse::Scalar(Location {
                uri: uri.clone(),
                range: DiagnosticConverter::new(source.to_string(), line_index.clone())
                    .span_to_range_indexed(range.start, range.end),
            })),
            CursorDefinition::OtherFile(loc) => {
                let (target_uri, target_range) = Self::def_location_to_lsp(&loc)?;
                Some(GotoDefinitionResponse::Link(vec![LocationLink {
                    origin_selection_range: None,
                    target_uri,
                    target_range,
                    target_selection_range: target_range,
                }]))
            }
            CursorDefinition::Import { target, origin } => {
                let (target_uri, target_range) = Self::import_nav_to_lsp(&target)?;
                let origin_selection_range = origin.map(|r| {
                    DiagnosticConverter::new(source.to_string(), line_index.clone())
                        .span_to_range_indexed(r.start, r.end)
                });
                Some(GotoDefinitionResponse::Link(vec![LocationLink {
                    origin_selection_range,
                    target_uri,
                    target_range,
                    target_selection_range: target_range,
                }]))
            }
        }
    }

    fn def_location_to_lsp(loc: &DefLocation) -> Option<(Url, Range)> {
        let source = std::fs::read_to_string(&loc.file).ok()?;
        let line_index = ast::source::LineIndex::new(&source);
        let converter = DiagnosticConverter::new(source, line_index);
        let range = converter.span_to_range_indexed(loc.byte_range.start, loc.byte_range.end);
        let uri = Url::from_file_path(&loc.file).ok()?;
        Some((uri, range))
    }

    fn import_nav_to_lsp(nav: &ImportNavLocation) -> Option<(Url, Range)> {
        match nav {
            ImportNavLocation::Manifest(path) | ImportNavLocation::FileRoot(path) => {
                let uri = Url::from_file_path(path).ok()?;
                Some((
                    uri,
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                ))
            }
            ImportNavLocation::Definition(loc) => Self::def_location_to_lsp(loc),
        }
    }
}
