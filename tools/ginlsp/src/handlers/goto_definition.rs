use crate::diagnostics::span_to_range;
use crate::Backend;
use ast::ImportSource;
use database::parse_file;
use ide::{find_definition_span, get_word_at_position, position_to_byte_offset};
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

        if let Some(state) = self.documents.get(&uri.to_string()) {
            let snapshot = self.snapshot();
            let ast = parse_file(&snapshot.db, state.file);

            let byte_pos = position_to_byte_offset(&state.source, position.line, position.character);

            // Check if cursor is inside a use import string or package path
            if let Some(byte_pos) = byte_pos {
                if let Some(link) = self.resolve_use_import_at(&uri, &ast, &state.source, byte_pos)
                {
                    return Ok(Some(GotoDefinitionResponse::Link(vec![link])));
                }
            }

            if let Some(word) =
                get_word_at_position(&state.source, position.line, position.character)
            {
                let range = find_definition_span(&ast, &word)
                    .map(|span| span_to_range(span.start, span.end, &state.source))
                    .unwrap_or_default();
                if range != Range::default() {
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri,
                        range,
                    })));
                }
            }
        }

        Ok(None)
    }
}

impl Backend {
    /// Check if `byte_pos` falls inside any import in the AST, and if so,
    /// resolve the import target and return a `LocationLink` whose
    /// `origin_selection_range` covers the full import string/path.
    fn resolve_use_import_at(
        &self,
        base_uri: &Url,
        ast: &ast::FileAst,
        source: &str,
        byte_pos: usize,
    ) -> Option<LocationLink> {
        let span_table = ast.span_table();

        for import in ast.uses() {
            for module_import in &import.0 {
                let (import_span, import_path) = match &module_import.source {
                    ImportSource::Local(path, span_id) => {
                        let _span = span_table.get(*span_id);
                        (*span_id, path.clone())
                    }
                    ImportSource::Package(mod_path) => {
                        let _span = span_table.get(mod_path.span);
                        // Reconstruct a package path string for resolution
                        let mut s = mod_path.root.as_str().to_string();
                        for seg in &mod_path.segments {
                            s.push('/');
                            s.push_str(seg.as_str());
                        }
                        (mod_path.span, std::path::PathBuf::from(s))
                    }
                };

                let span = span_table.get(import_span);
                if byte_pos < span.start || byte_pos > span.end {
                    continue;
                }

                let origin_range = span_to_range(span.start, span.end, source);

                if let Some(target_location) =
                    self.resolve_use_import(base_uri, &import_path)
                {
                    return Some(LocationLink {
                        origin_selection_range: Some(origin_range),
                        target_uri: target_location.uri,
                        target_range: target_location.range,
                        target_selection_range: target_location.range,
                    });
                }
            }
        }

        None
    }

    /// Resolve a use import path to a file location.
    ///
    /// Tries to open flask.jsonc for the module if available,
    /// otherwise opens the first .gin file found in the module.
    ///
    /// TODO: Evaluate this behavior in the future. When a module has multiple files,
    /// we might want to show a list of options or navigate to main.gin if it exists.
    fn resolve_use_import(&self, base_uri: &Url, import_path: &std::path::Path) -> Option<Location> {
        let base_path = base_uri.to_file_path().ok()?;
        let base_dir = base_path.parent()?;

        let resolved_path = base_dir.join(import_path);

        // Check if it's a directory (module)
        if resolved_path.is_dir() {
            // Try to open flask.jsonc first
            let flask_jsonc_path = resolved_path.join(flask::PACKAGE_CONFIG_NAME);
            if flask_jsonc_path.exists() {
                return Some(Location {
                    uri: Url::from_file_path(&flask_jsonc_path).ok()?,
                    range: Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                });
            }

            // Fall back to finding the first .gin file
            if let Ok(entries) = std::fs::read_dir(&resolved_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map_or(false, |e| e == "gin") {
                        return Some(Location {
                            uri: Url::from_file_path(&path).ok()?,
                            range: Range {
                                start: Position {
                                    line: 0,
                                    character: 0,
                                },
                                end: Position {
                                    line: 0,
                                    character: 0,
                                },
                            },
                        });
                    }
                }
            }
        } else if resolved_path.exists() {
            return Some(Location {
                uri: Url::from_file_path(&resolved_path).ok()?,
                range: Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                },
            });
        }

        None
    }
}
