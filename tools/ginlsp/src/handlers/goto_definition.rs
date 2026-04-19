use crate::diagnostics::span_to_range;
use crate::Backend;
use lsp::{find_definition_span, get_word_at_position};
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
            let ast = snapshot.parse(state.file);

            // Check if cursor is on a string in a use statement
            let line_text = state.source.lines().nth(position.line as usize).unwrap_or("");
            let trimmed = line_text.trim_start();
            
            if trimmed.starts_with("use ") {
                let col = position.character as usize;
                let before_cursor = &line_text[..col.min(line_text.len())];
                
                if let Some(quote_pos) = before_cursor.rfind('\'') {
                    let partial = &before_cursor[quote_pos + 1..];
                    
                    // Resolve the module path and try to navigate
                    if let Some(location) = self.resolve_use_import(&uri, partial, position) {
                        return Ok(Some(GotoDefinitionResponse::Scalar(location)));
                    }
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
    /// Resolve a use import path to a file location.
    ///
    /// Tries to open flask.jsonc for the module if available,
    /// otherwise opens the first .gin file found in the module.
    ///
    /// TODO: Evaluate this behavior in the future. When a module has multiple files,
    /// we might want to show a list of options or navigate to main.gin if it exists.
    fn resolve_use_import(
        &self,
        base_uri: &Url,
        import_path: &str,
        _position: Position,
    ) -> Option<Location> {
        let base_path = base_uri.to_file_path().ok()?;
        let base_dir = base_path.parent()?;
        
        // Resolve the import path relative to base directory
        let resolved_path = base_dir.join(import_path);
        
        // Check if it's a directory (module)
        if resolved_path.is_dir() {
            // Try to open flask.jsonc first
            let flask_jsonc_path = resolved_path.join(flask::PACKAGE_CONFIG_NAME);
            if flask_jsonc_path.exists() {
                return Some(Location {
                    uri: Url::from_file_path(&flask_jsonc_path).ok()?,
                    range: Range {
                        start: Position { line: 0, character: 0 },
                        end: Position { line: 0, character: 0 },
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
                                start: Position { line: 0, character: 0 },
                                end: Position { line: 0, character: 0 },
                            },
                        });
                    }
                }
            }
        } else if resolved_path.exists() {
            // Direct file reference
            return Some(Location {
                uri: Url::from_file_path(&resolved_path).ok()?,
                range: Range {
                    start: Position { line: 0, character: 0 },
                    end: Position { line: 0, character: 0 },
                },
            });
        }
        
        None
    }
}
