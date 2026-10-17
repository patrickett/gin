use crate::Backend;
use crate::state::ManifestDocumentState;
use tower_lsp::lsp_types::*;

impl Backend {
    fn note_manifest_document(&self, uri: String, source: String) {
        self.manifest_documents
            .insert(uri, ManifestDocumentState { source });
    }

    pub(crate) async fn handle_did_open(&self, params: DidOpenTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let text = params.text_document.text.clone();

        if Backend::is_manifest_file(&params.text_document.uri) {
            self.publish_manifest_diagnostics(params.text_document.uri.clone(), &text)
                .await;
            self.note_manifest_document(uri.clone(), text);
            self.client
                .log_message(MessageType::INFO, format!("Opened mod.flask: {}", uri))
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
        self.debug_log(format!("did open: {:#?}", params)).await;
    }

    pub(crate) async fn handle_did_change(&self, params: DidChangeTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();

        if Backend::is_manifest_file(&params.text_document.uri) {
            if let Some(change) = params.content_changes.first() {
                self.publish_manifest_diagnostics(params.text_document.uri.clone(), &change.text)
                    .await;
                self.note_manifest_document(uri, change.text.clone());
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
        self.debug_log(format!("did change: {:#?}", params)).await;
    }

    pub(crate) async fn handle_did_save(&self, params: DidSaveTextDocumentParams) {
        if !Backend::should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();

        if Backend::is_manifest_file(&params.text_document.uri) {
            if let Some(path) = Self::file_path_from_uri(&params.text_document.uri) {
                self.package_configs.clear();
                self.lock_host().cache.invalidate_package_cache_for(&path);
            }
            if let Some(ref text) = params.text {
                self.publish_manifest_diagnostics(params.text_document.uri.clone(), text)
                    .await;
                self.note_manifest_document(uri, text.clone());
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
        self.debug_log(format!("file saved: {:#?}", params)).await;
    }

    pub(crate) async fn handle_did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        if self.manifest_documents.remove(&uri).is_none() {
            // .gin file closed: evict cached file/module state so closed
            // documents can drop parse memory while active files keep
            // responsive diagnostics via recomputation.
            if let Some(path) = Backend::file_path_from_uri(&params.text_document.uri) {
                let mut host = self.lock_host();
                host.evict_file_for(&path);
            }
        }

        #[cfg(debug_assertions)]
        self.debug_log(format!("did close: {:#?}", params)).await;
    }
}
