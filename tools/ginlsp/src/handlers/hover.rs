use crate::Backend;
use lsp::{
    byte_offset_to_position, get_char_at_position, get_number_at_position, get_string_literal_at,
    hover_at, is_in_comment, position_to_byte_offset, word_at_byte_offset,
};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_hover(&self, params: HoverParams) -> Result<Option<Hover>> {
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
            if is_in_comment(&state.source, position.line, position.character) {
                return Ok(None);
            }

            if let Some('(' | ')' | '[' | ']') =
                get_char_at_position(&state.source, position.line, position.character)
            {
                return Ok(None);
            }

            if let Some(byte_pos) =
                position_to_byte_offset(&state.source, position.line, position.character)
            {
                // Number literals
                if let Some(num) = get_number_at_position(
                    &state.source,
                    position.line,
                    position.character,
                ) {
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!("```gin\n{num}\n```"),
                        }),
                        range: None,
                    }));
                }

                // String literals — hover shows the whole token
                if let Some(info) = get_string_literal_at(&state.source, byte_pos) {
                    let (start_line, start_char) =
                        byte_offset_to_position(info.range.start, &state.source);
                    let (end_line, end_char) =
                        byte_offset_to_position(info.range.end, &state.source);
                    let range = Range {
                        start: Position {
                            line: start_line,
                            character: start_char,
                        },
                        end: Position {
                            line: end_line,
                            character: end_char,
                        },
                    };
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!(
                                "```gin\nvalue of literal: '{}'\n```",
                                info.content
                            ),
                        }),
                        range: Some(range),
                    }));
                }

                // Keyword hover — `use`
                if let Some(word) = word_at_byte_offset(&state.source, byte_pos) {
                    if word == "use" {
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: String::from(
                                    "```gin\nuse\n```\n\n\
                                    ---\n\n\
                                    Import items from other modules.\n\n\
                                    **Local imports** — single-quoted path relative to the current file:\n\n\
                                    ```gin\n\
                                    use './math' as math\n\
                                    use '../shared/utils'\n\
                                    ```\n\n\
                                    The path resolves to a module folder. All `.gin` files inside are \
                                    included. Do not include the `.gin` extension.\n\n\
                                    **Package imports** — dotted name from `flask.jsonc` dependencies:\n\n\
                                    ```gin\n\
                                    use core.io\n\
                                    use http.web as web\n\
                                    ```\n\n\
                                    The root name (`core`, `http`) must be declared in `flask.jsonc` \
                                    under `dependencies`. Subsequent segments resolve to files within \
                                    the dependency directory.",
                                ),
                            }),
                            range: None,
                        }));
                    }
                }

                // General hover (definitions, bindings, etc.)
                let snapshot = self.snapshot();
                let ast = snapshot.parse(state.file);
                if let Some(value) = hover_at(&state.source, &ast, byte_pos) {
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value,
                        }),
                        range: None,
                    }));
                }
            }
        }
        Ok(None)
    }
}
