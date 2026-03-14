mod capabilities;
mod diagnostics;
mod state;
mod util;

use capabilities::{
    build_completions, build_hover, build_semantic_tokens_from_ast,
    build_signature_help, build_variant_hover, complete_flask_json, find_all_references, find_definition_range,
    is_flask_json_file, should_handle_file, use_completions, LEGEND_TYPE,
};
use dashmap::DashMap;
use diagnostics::symptoms_to_diagnostics;
use state::{DocumentState, GinHost, JsonDocumentState};
use std::sync::{Arc, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use util::get_word_at_position;

const INFO: MessageType = MessageType::INFO;

struct Backend {
    client: Client,
    host: Arc<std::sync::Mutex<GinHost>>,
    documents: DashMap<String, DocumentState>,
    json_documents: DashMap<String, JsonDocumentState>,
    config: RwLock<Option<flask::FlaskConfigHandle>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            host: Arc::new(std::sync::Mutex::new(GinHost::new())),
            documents: DashMap::new(),
            json_documents: DashMap::new(),
            config: RwLock::new(None),
        }
    }

    fn snapshot(&self) -> state::GinSnapshot {
        let host = self.host.lock().unwrap();
        host.snapshot()
    }

    /// Get or load config for a file's project.
    /// Caches the config handle for reuse.
    fn get_or_load_config(&self, file_uri: &Url) -> Option<flask::FlaskConfigHandle> {
        // First, try to read existing config
        {
            let config = self.config.read().unwrap();
            if config.is_some() {
                return config.clone();
            }
        }

        // Load config from the file's directory
        let file_path = file_uri.to_file_path().ok()?;
        let file_dir = file_path.parent()?;

        if let Ok(handle) = flask::FlaskConfigHandle::load(file_dir) {
            let mut config = self.config.write().unwrap();
            *config = Some(handle.clone());
            return Some(handle);
        }

        None
    }

    /// Compute the module path for a file, relative to its flask.json project root.
    /// Modules are folders, not files, so we drop the filename component.
    fn compute_module_path(&self, file_path: &std::path::Path, uri: &Url) -> String {
        if let Some(handle) = self.get_or_load_config(uri) {
            let source_dir = handle.source_dir();
            let package_name = handle.read().name().to_string();

            let relative = file_path.strip_prefix(&source_dir).unwrap_or(file_path);
            let without_ext = relative.with_extension("");
            let segments: Vec<&str> = without_ext
                .components()
                .filter_map(|c| match c {
                    std::path::Component::Normal(s) => s.to_str(),
                    _ => None,
                })
                .collect();

            // Drop the filename (last segment) - modules are folders
            let module_segments = if segments.len() > 1 {
                &segments[..segments.len() - 1]
            } else {
                &segments
            };

            if module_segments.is_empty() {
                package_name
            } else {
                format!("{}.{}", package_name, module_segments.join("."))
            }
        } else {
            // fallback: file stem only (no flask.json found)
            file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string()
        }
    }

    async fn publish_diagnostics_for(&self, uri: Url, file: ginc::File, source: &str) {
        let snapshot = self.snapshot();
        let _ast = snapshot.parse(file);
        let symptoms = snapshot.diagnostics(file);
        let diagnostics = symptoms_to_diagnostics(source, &symptoms[..]);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        // TODO: Consider dynamic registration for proper document filtering.
        // Static capabilities don't support document selectors for completion/hover/etc.
        // Dynamic registration via `client/registerCapability` allows per-capability filtering.
        // See: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#client_registerCapability
        let gin_file_doc_filter = DocumentFilter {
            language: Some("gin".to_string()),
            scheme: Some("file".to_string()),
            pattern: None,
        };

        let capabilities = ServerCapabilities {
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
                    SemanticTokensRegistrationOptions {
                        text_document_registration_options: TextDocumentRegistrationOptions {
                            document_selector: Some(vec![gin_file_doc_filter.clone()]),
                        },
                        semantic_tokens_options: SemanticTokensOptions {
                            work_done_progress_options: WorkDoneProgressOptions::default(),
                            legend: SemanticTokensLegend {
                                token_types: LEGEND_TYPE.into(),
                                token_modifiers: vec![],
                            },
                            range: Some(true),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                        static_registration_options: StaticRegistrationOptions::default(),
                    },
                ),
            ),
            text_document_sync: Some(TextDocumentSyncCapability::Options(
                TextDocumentSyncOptions {
                    open_close: Some(true),
                    change: Some(TextDocumentSyncKind::FULL),
                    save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                        include_text: Some(true),
                    })),
                    ..Default::default()
                },
            )),
            definition_provider: Some(OneOf::Left(true)),
            references_provider: Some(OneOf::Left(true)),
            completion_provider: Some(CompletionOptions {
                resolve_provider: Some(false),
                trigger_characters: Some(vec![
                    ".".to_string(),
                    "'".to_string(),
                    "/".to_string(), // Gin
                    ":".to_string(),
                    "\"".to_string(), // JSON
                ]),
                all_commit_characters: Some(vec![
                    ":".to_string(),
                    ",".to_string(),
                    "\"".to_string(),
                    "}".to_string(),
                ]),
                ..Default::default()
            }),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: Some(vec!["(".to_string()]),
                retrigger_characters: Some(vec![",".to_string()]),
                work_done_progress_options: WorkDoneProgressOptions::default(),
            }),
            ..Default::default()
        };

        Ok(InitializeResult {
            capabilities,
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(INFO, "gin language server initialized!")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        // Filter out files we don't handle
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let text = params.text_document.text.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        // Check if this is a flask.json file
        if is_flask_json_file(&params.text_document.uri) {
            self.json_documents.insert(
                uri.clone(),
                JsonDocumentState {
                    source: text.clone(),
                },
            );
            self.client
                .log_message(INFO, format!("Opened flask.json: {}", uri))
                .await;
            return;
        }

        // Handle Gin files
        let file = {
            let mut host = self.host.lock().unwrap();
            host.upsert_file(path, text.clone())
        };

        if let Some(file) = file {
            self.documents.insert(
                uri.clone(),
                DocumentState {
                    source: text.clone(),
                    file,
                },
            );
            self.publish_diagnostics_for(uri_for_diag, file, &text)
                .await;
        }

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("did open: {:#?}", params))
                .await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // Filter out files we don't handle
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        // Check if this is a flask.json file
        if is_flask_json_file(&params.text_document.uri) {
            if let Some(change) = params.content_changes.first() {
                let text = change.text.clone();
                self.json_documents.insert(
                    uri.clone(),
                    JsonDocumentState {
                        source: text.clone(),
                    },
                );
            }
            return;
        }

        // Handle Gin files
        if let Some(change) = params.content_changes.first() {
            let text = change.text.clone();

            let file = {
                let mut host = self.host.lock().unwrap();
                host.upsert_file(path, text.clone())
            };

            if let Some(file) = file {
                self.documents.insert(
                    uri.clone(),
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
        {
            self.client
                .log_message(INFO, format!("did change: {:#?}", params))
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        // Filter out files we don't handle
        if !should_handle_file(&params.text_document.uri) {
            return;
        }

        let uri = params.text_document.uri.to_string();
        let uri_for_diag = params.text_document.uri.clone();
        let path = params.text_document.uri.to_file_path().unwrap_or_default();

        // Check if this is a flask.json file
        if is_flask_json_file(&params.text_document.uri) {
            if let Some(text) = &params.text {
                let text = text.clone();
                self.json_documents.insert(
                    uri.clone(),
                    JsonDocumentState {
                        source: text.clone(),
                    },
                );
            }
            return;
        }

        // Handle Gin files
        if let Some(text) = &params.text {
            let text = text.clone();

            let file = {
                let mut host = self.host.lock().unwrap();
                host.upsert_file(path, text.clone())
            };

            if let Some(file) = file {
                self.documents.insert(
                    uri.clone(),
                    DocumentState {
                        source: text.clone(),
                        file,
                    },
                );
                let _ = self.client.semantic_tokens_refresh().await;
                self.publish_diagnostics_for(uri_for_diag, file, &text)
                    .await;
            }
        }

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("file saved: {:#?}", params))
                .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri.to_string();

        // Remove from either documents or json_documents
        if self.json_documents.remove(&uri).is_some() {
            // Was a JSON file
        } else {
            self.documents.remove(&uri);
        }

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("did close: {:#?}", params))
                .await;
        }
    }

    async fn shutdown(&self) -> Result<()> {
        self.client
            .log_message(INFO, "gin language server shutting down!")
            .await;
        Ok(())
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.to_string();

        if let Some(state) = self.documents.get(&uri) {
            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);
            let semantic_tokens = build_semantic_tokens_from_ast(&state.source, &ast);
            return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: semantic_tokens,
            })));
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);

            if let Some(word) = get_word_at_position(&state.source, position) {
                if ast.tags().keys().any(|t| t.as_str() == word) {
                    let range = find_definition_range(&state.source, &word, true);
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri,
                        range,
                    })));
                }

                if ast.defs().keys().any(|d| d.as_str() == word) {
                    let range = find_definition_range(&state.source, &word, false);
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri,
                        range,
                    })));
                }
            }
        }

        Ok(None)
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let position = params.text_document_position.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            if let Some(word) = get_word_at_position(&state.source, position) {
                let locations = find_all_references(&state.source, &word, &uri);
                if !locations.is_empty() {
                    return Ok(Some(locations));
                }
            }
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let position = params.text_document_position.position;

        // Debug: log completion request
        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("completion requested for URI: {}", uri))
                .await;
        }

        // Check if this is a flask.json file
        if let Some(state) = self.json_documents.get(&uri) {
            let items = complete_flask_json(
                &state.source,
                position,
                &params.text_document_position.text_document.uri,
            );
            #[cfg(debug_assertions)]
            {
                self.client
                    .log_message(
                        INFO,
                        format!("Returning {} completions for flask.json", items.len()),
                    )
                    .await;
            }
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // Handle Gin files
        if let Some(state) = self.documents.get(&uri) {
            let config = self.get_or_load_config(&params.text_document_position.text_document.uri);

            if let Some(items) = use_completions(
                &state.source,
                position,
                &params.text_document_position.text_document.uri,
                config.as_ref(),
            ) {
                return Ok(Some(CompletionResponse::Array(items)));
            }

            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);
            let items = build_completions(&ast);

            return Ok(Some(CompletionResponse::Array(items)));
        }

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("No document found for URI: {}", uri))
                .await;
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        if let Some(state) = self.documents.get(&uri.to_string()) {
            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);
            let file_path = state.file.path(&snapshot.db);
            let module = self.compute_module_path(&file_path, &uri);

            if let Some(word) = get_word_at_position(&state.source, position) {
                // First check if it's a variant within a union
                use ginc::ast::{DeclareValue, Variant};
                for (name, decl) in ast.tags() {
                    if let DeclareValue::Union { variants } = decl.value() {
                        for variant in variants {
                            let (tag, doc) = match variant {
                                Variant::External(tag) => (tag, None),
                                Variant::Local { tag, doc_comment } => {
                                    (tag, doc_comment.as_ref())
                                }
                            };
                            if tag.name() == word {
                                // Found variant within union
                                let value =
                                    build_variant_hover(&module, &word, name.as_str(), doc);
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
                }

                // Then check top-level tags
                for (name, decl) in ast.tags() {
                    if name.as_str() == word {
                        let doc = decl.doc_comment();
                        let value = build_hover(
                            &state.source,
                            &module,
                            &word,
                            true,
                            &doc.cloned(),
                        );
                        return Ok(Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value,
                            }),
                            range: None,
                        }));
                    }
                }

                // Then check bindings
                for (name, _bind) in ast.defs() {
                    if name.as_str() == word {
                        let value = build_hover(
                            &state.source,
                            &module,
                            &word,
                            false,
                            &None,  // Don't show doc comments in hover
                        );
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
        }

        Ok(None)
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .to_string();
        let position = params.text_document_position_params.position;

        if let Some(state) = self.documents.get(&uri) {
            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);

            if let Some(help) = build_signature_help(&state.source, &ast, position) {
                return Ok(Some(help));
            }
        }

        Ok(None)
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
