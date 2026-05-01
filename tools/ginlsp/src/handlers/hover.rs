use crate::Backend;
use database::semantic_queries::hover_markdown;
use typeck::{
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

        // Snapshot source + file id, drop DashMap ref before any spawn_blocking
        // await: `Ref` holds a shard read-lock and is `!Send`.
        let (source, file) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file),
            None => return Ok(None),
        };

        if is_in_comment(&source, position.line, position.character) {
            return Ok(None);
        }

        if let Some('(' | ')' | '[' | ']') =
            get_char_at_position(&source, position.line, position.character)
        {
            return Ok(None);
        }

        let Some(byte_pos) = position_to_byte_offset(&source, position.line, position.character)
        else {
            return Ok(None);
        };

        let dot_hover_range = compute_dot_hover_range(&source, byte_pos);

        if let Some(range_lit) =
            get_range_literal_at_position(&source, position.line, position.character)
        {
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

        if let Some(num) = get_number_at_position(&source, position.line, position.character) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```gin\n{num}\n```"),
                }),
                range: None,
            }));
        }

        if let Some(info) = get_string_literal_at(&source, byte_pos) {
            let (start_line, start_char) = byte_offset_to_position(info.range.start, &source);
            let (end_line, end_char) = byte_offset_to_position(info.range.end, &source);
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
                    value: format!("```gin\nvalue of literal: '{}'\n```", info.content),
                }),
                range: Some(range),
            }));
        }

        if let Some(word) = word_at_byte_offset(&source, byte_pos) {
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

        // General hover hits hover_markdown, which runs the parser via Salsa.
        // Off-load to the blocking pool so a stuck parse cannot pin the runtime.
        let Some(byte_pos_u32) = u32::try_from(byte_pos).ok() else {
            return Ok(None);
        };
        let value = self
            .run_blocking_request("hover", move |this| {
                let snapshot = this.snapshot();
                hover_markdown(&snapshot.db, file, byte_pos_u32)
            })
            .await
            .flatten();

        Ok(value.map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }
}

fn compute_dot_hover_range(source: &str, byte_pos: usize) -> Option<Range> {
    let bytes = source.as_bytes();
    if bytes.get(byte_pos) != Some(&b'.') {
        return None;
    }

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
    } else if bytes.get(byte_pos + 1) == Some(&b'.') && bytes.get(byte_pos + 2) == Some(&b'.') {
        Some(byte_pos)
    } else {
        None
    }?;

    let (start_line, start_char) = byte_offset_to_position(start, source);
    let (end_line, end_char) = byte_offset_to_position(start + 3, source);
    Some(Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    })
}
