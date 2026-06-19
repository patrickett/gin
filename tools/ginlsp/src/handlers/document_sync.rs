use crate::Backend;
use crate::state::JsonDocumentState;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_did_open(&self, params: DidOpenTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let text = params.text_document.text.clone();

        if Backend::is_flask_json_file(&params.text_document.uri) {
            self.json_documents
                .insert(uri.clone(), JsonDocumentState { source: text });
            self.client
                .log_message(MessageType::INFO, format!("Opened flask.jsonc: {}", uri))
                .await;
            return;
        }

        let Some(file_path) = Backend::file_path_from_uri(&params.text_document.uri) else {
            return;
        };

        {
            let mut host = self.lock_host();
            host.upsert_file(file_path.clone(), text);
        }
        self.spawn_publish_diagnostics(uri_for_diag, file_path);

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did open: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_change(&self, params: DidChangeTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();

        if Backend::is_flask_json_file(&params.text_document.uri) {
            if let Some(change) = params.content_changes.first() {
                self.json_documents.insert(
                    uri,
                    JsonDocumentState {
                        source: change.text.clone(),
                    },
                );
            }
            return;
        }

        let Some(path) = Backend::file_path_from_uri(&params.text_document.uri) else {
            return;
        };

        if let Some(change) = params.content_changes.first() {
            let text = change.text.clone();
            {
                let mut host = self.lock_host();
                host.upsert_file(path.clone(), text);
            }
            self.spawn_publish_diagnostics(uri_for_diag, path);
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did change: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_save(&self, params: DidSaveTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();

        if Backend::is_flask_json_file(&params.text_document.uri) {
            if let Some(ref text) = params.text {
                self.json_documents.insert(
                    uri,
                    JsonDocumentState {
                        source: text.clone(),
                    },
                );
            }
            return;
        }

        let Some(path) = Backend::file_path_from_uri(&params.text_document.uri) else {
            return;
        };

        if let Some(ref text) = params.text {
            {
                let mut host = self.lock_host();
                host.upsert_file(path.clone(), text.clone());
            }
            self.spawn_publish_diagnostics(uri_for_diag, path);
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("file saved: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        if self.json_documents.remove(&uri).is_none() {
            // .gin file closed: evict the cached package entry to free typed
            // AST memory. The file's source + parse output remain cached so
            // cross-file diagnostics still work for any still-open files in
            // the same package. The entry is recomputed on next access.
            if let Some(path) = Backend::file_path_from_uri(&params.text_document.uri) {
                let host = self.lock_host();
                host.evict_package_for(&path);
            }
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did close: {:#?}", params))
            .await;
    }
}
