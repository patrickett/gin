use crate::handlers::{is_flask_json_file, should_handle_file};
use crate::state::{DocumentState, JsonDocumentState};
use crate::Backend;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_did_open(&self, params: DidOpenTextDocumentParams) {
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        if is_flask_json_file(&params.text_document.uri) {
            self.json_documents
                .insert(uri.clone(), JsonDocumentState { source: text });
            self.client
                .log_message(MessageType::INFO, format!("Opened flask.json: {}", uri))
                .await;
            return;
        }

        let file = {
            let mut host = self.host.lock().unwrap();
            host.upsert_file(path, text.clone())
        };

        if let Some(file) = file {
            self.documents.insert(
                uri,
                DocumentState {
                    source: text.clone(),
                    file,
                },
            );
            self.publish_diagnostics_for(uri_for_diag, file, &text)
                .await;
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did open: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_change(&self, params: DidChangeTextDocumentParams) {
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        if is_flask_json_file(&params.text_document.uri) {
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

        if let Some(change) = params.content_changes.first() {
            let text = change.text.clone();
            let file = {
                let mut host = self.host.lock().unwrap();
                host.upsert_file(path, text.clone())
            };

            if let Some(file) = file {
                self.documents.insert(
                    uri,
                    DocumentState {
                        source: text.clone(),
                        file,
                    },
                );
                self.publish_diagnostics_for(uri_for_diag, file, &text)
                    .await;
            }
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did change: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_save(&self, params: DidSaveTextDocumentParams) {
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        if is_flask_json_file(&params.text_document.uri) {
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

        if let Some(ref text) = params.text {
            let file = {
                let mut host = self.host.lock().unwrap();
                host.upsert_file(path, text.clone())
            };

            if let Some(file) = file {
                self.documents.insert(
                    uri,
                    DocumentState {
                        source: text.clone(),
                        file,
                    },
                );
                self.publish_diagnostics_for(uri_for_diag, file, text).await;
            }
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("file saved: {:#?}", params))
            .await;
    }

    pub(crate) async fn handle_did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        if self.json_documents.remove(&uri).is_none() {
            self.documents.remove(&uri);
            self.ast_cache.remove(&uri);
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(MessageType::INFO, format!("did close: {:#?}", params))
            .await;
    }
}
