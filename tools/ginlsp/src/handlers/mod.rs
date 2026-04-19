pub(crate) mod completion;
mod document_sync;
mod formatting;
mod goto_definition;
mod hover;
mod lifecycle;
mod references;
mod signature_help;

use tower_lsp::lsp_types::Url;

pub(crate) fn is_flask_json_file(uri: &Url) -> bool {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy() == "flask.jsonc"))
        .unwrap_or(false)
}

fn is_gin_file(uri: &Url) -> bool {
    uri.to_file_path()
        .ok()
        .and_then(|p| p.extension().map(|e| e.to_string_lossy() == "gin"))
        .unwrap_or(false)
}

pub(crate) fn should_handle_file(uri: &Url) -> bool {
    is_gin_file(uri) || is_flask_json_file(uri)
}
