pub(crate) mod code_action;
pub(crate) mod completion;
mod document_sync;
pub(crate) mod formatting;
mod goto_definition;
mod hover;
mod lifecycle;
mod manifest;
mod references;
mod signature_help;
mod style_actions;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::Backend;
use analysis::DocumentSnapshot;
use ast::source::SourceExt;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, Position, Range, TextDocumentPositionParams, TextEdit, Url,
    WorkspaceEdit,
};

pub(crate) fn byte_range_to_range(start: usize, end: usize, source: &str) -> Range {
    let (sl, sc) = source.byte_offset_to_position(start);
    let (el, ec) = source.byte_offset_to_position(end);
    Range {
        start: Position {
            line: sl,
            character: sc,
        },
        end: Position {
            line: el,
            character: ec,
        },
    }
}

pub(crate) fn line_byte_range(source: &str, line: u32) -> Option<(usize, usize)> {
    let start = source.position_to_byte_offset(line, 0)?;
    let end = source
        .position_to_byte_offset(line + 1, 0)
        .unwrap_or(source.len());
    Some((start, end))
}

pub(crate) struct RequestTarget {
    pub(crate) uri: Url,
    pub(crate) file_path: PathBuf,
    pub(crate) position: Position,
}

pub(crate) fn request_target(params: &TextDocumentPositionParams) -> Option<RequestTarget> {
    let uri = params.text_document.uri.clone();
    let file_path = Backend::file_path_from_uri(&uri)?;
    Some(RequestTarget {
        uri,
        file_path,
        position: params.position,
    })
}

pub(crate) fn byte_position(doc: &DocumentSnapshot, position: Position) -> Option<usize> {
    doc.line_index
        .position_to_byte(&doc.source, position.line, position.character)
}

pub(crate) fn single_file_action(
    uri: &Url,
    title: String,
    kind: CodeActionKind,
    edits: Vec<TextEdit>,
) -> CodeAction {
    CodeAction {
        title,
        kind: Some(kind),
        edit: Some(WorkspaceEdit {
            changes: Some(HashMap::from([(uri.clone(), edits)])),
            ..Default::default()
        }),
        is_preferred: Some(true),
        ..Default::default()
    }
}

impl Backend {
    #[cfg(debug_assertions)]
    pub(crate) async fn debug_log(&self, message: String) {
        self.client
            .log_message(tower_lsp::lsp_types::MessageType::INFO, message)
            .await;
    }

    pub(crate) fn is_manifest_file(uri: &Url) -> bool {
        uri.to_file_path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == flask::MANIFEST_NAME))
            .unwrap_or(false)
    }

    fn is_gin_file(uri: &Url) -> bool {
        uri.to_file_path()
            .ok()
            .and_then(|p| p.extension().map(|e| e.to_string_lossy() == "gin"))
            .unwrap_or(false)
    }

    pub(crate) fn should_handle_file(uri: &Url) -> bool {
        Self::is_gin_file(uri) || Self::is_manifest_file(uri)
    }
}
