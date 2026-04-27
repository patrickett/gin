use crate::Backend;
use analyze::hover_markdown;
use ide::{
    byte_offset_to_position, get_char_at_position, get_number_at_position, get_string_literal_at,
    get_range_literal_at_position, is_in_comment, position_to_byte_offset, word_at_byte_offset,
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
                // TODO: handle the ... hover with info about range and link to range.gin
                // TODO: also handle the number hover with info about the number and link to number.gin, 
                // auto detect the size of the int and and in for loops bind that type to the loop variable
                let dot_hover_range = {
                    let bytes = state.source.as_bytes();
                    let is_dot = bytes.get(byte_pos) == Some(&b'.');
                    if !is_dot {
                        None
                    } else {
                        let start = if byte_pos >= 2
                            && bytes.get(byte_pos - 2) == Some(&b'.')
                            && bytes.get(byte_pos - 1) == Some(&b'.')
                        {
                            Some(byte_pos - 2)
                        } else if byte_pos >= 1
                            && bytes.get(byte_pos - 1) == Some(&b'.')
                            && bytes.get(byte_pos + 1) == Some(&b'.')
                        {
                            Some(byte_pos - 1)
                        } else if bytes.get(byte_pos + 1) == Some(&b'.')
                            && bytes.get(byte_pos + 2) == Some(&b'.')
                        {
                            Some(byte_pos)
                        } else {
                            None
                        };

                        start.map(|s| {
                            let (start_line, start_char) =
                                byte_offset_to_position(s, &state.source);
                            let (end_line, end_char) =
                                byte_offset_to_position(s + 3, &state.source);
                            Range {
                                start: Position {
                                    line: start_line,
                                    character: start_char,
                                },
                                end: Position {
                                    line: end_line,
                                    character: end_char,
                                },
                            }
                        })
                    }
                };

                // Range literals (e.g. `12...1200`) — hover shows the whole token
                if let Some(range_lit) = get_range_literal_at_position(
                    &state.source,
                    position.line,
                    position.character,
                ) {
                    if dot_hover_range.is_some() {
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: String::from(
                                    "```gin\n...\n```\n\n---\n\n\
                                    Creates a `core.range.Range` from `start...end`.\n\n\
                                    - `start`: lower bound\n\
                                    - `end`: upper bound\n\n\
                                    Example:\n\n\
                                    ```gin\n\
                                    r Range(Int) := 12...1200\n\
                                    ```",
                                ),
                            }),
                            range: dot_hover_range,
                        }));
                    }
                    return Ok(Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!("```gin\n{range_lit}\n```"),
                        }),
                        range: dot_hover_range,
                    }));
                }

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
                let byte_pos_u32 = u32::try_from(byte_pos).ok();
                if let Some(value) = byte_pos_u32
                    .and_then(|pos| hover_markdown(&snapshot.db, state.file, pos))
                {
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
