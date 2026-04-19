mod diagnostics;
mod handlers;
mod state;

use ast::FileAst;
use dashmap::DashMap;
use database::File;
use database::Symptoms;
use diagnostic::lex::LexSymptom;
use diagnostic::parse::ParseSymptom;
use diagnostic::SymptomLike;
use diagnostics::symptoms_to_diagnostics;
use futures::FutureExt;
use parser::parse_source_full;
use state::{DocumentState, GinHost, JsonDocumentState};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::ops::Deref;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use typeck::analyze_file;

/// Shared LSP state. Heavy work (package diagnostics) is spawned so `shutdown`
/// and other requests are not stuck behind `did_change` / `did_open`.
pub(crate) struct GinLspBackend {
    pub(crate) client: Client,
    pub(crate) host: Arc<Mutex<GinHost>>,
    pub(crate) documents: DashMap<String, DocumentState>,
    pub(crate) json_documents: DashMap<String, JsonDocumentState>,
    pub(crate) config: RwLock<Option<flask::FlaskConfigHandle>>,
    pub(crate) ast_cache: DashMap<String, Arc<FileAst>>,
    pub(crate) shutdown: AtomicBool,
    /// Latest in-flight `publish_diagnostics_for` task; aborted on new publish and on shutdown.
    pub(crate) diagnostic_job: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone)]
pub(crate) struct Backend(Arc<GinLspBackend>);

impl Deref for Backend {
    type Target = GinLspBackend;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Backend {
    pub(crate) fn new(client: Client) -> Self {
        Self(Arc::new(GinLspBackend {
            client,
            host: Arc::new(Mutex::new(GinHost::new())),
            documents: DashMap::new(),
            json_documents: DashMap::new(),
            config: RwLock::new(None),
            ast_cache: DashMap::new(),
            shutdown: AtomicBool::new(false),
            diagnostic_job: Mutex::new(None),
        }))
    }

    /// Update `documents` / host synchronously, then run diagnostics without blocking LSP I/O.
    pub(crate) fn spawn_publish_diagnostics(&self, uri: Url, file: File, text: String) {
        let mut slot = self.diagnostic_job.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = slot.take() {
            prev.abort();
        }
        let this = self.clone();
        *slot = Some(tokio::spawn(async move {
            this.publish_diagnostics_for(uri, file, &text).await;
        }));
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn lock_host(&self) -> std::sync::MutexGuard<'_, GinHost> {
        self.host.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read_config(&self) -> std::sync::RwLockReadGuard<'_, Option<flask::FlaskConfigHandle>> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_config(&self) -> std::sync::RwLockWriteGuard<'_, Option<flask::FlaskConfigHandle>> {
        self.config.write().unwrap_or_else(|e| e.into_inner())
    }

    async fn catch_request<T: Send + 'static>(
        &self,
        name: &str,
        fut: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        AssertUnwindSafe(fut)
            .catch_unwind()
            .await
            .unwrap_or_else(|payload| {
                let msg = payload
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| payload.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                eprintln!("[ginlsp] handler '{}' panicked: {}", name, msg);
                Err(tower_lsp::jsonrpc::Error::internal_error())
            })
    }

    async fn catch_notification(&self, name: &str, fut: impl std::future::Future<Output = ()>) {
        if let Err(payload) = AssertUnwindSafe(fut).catch_unwind().await {
            let msg = payload
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            eprintln!("[ginlsp] notification handler '{}' panicked: {}", name, msg);
        }
    }

    fn snapshot(&self) -> state::GinSnapshot {
        let host = self.lock_host();
        host.snapshot()
    }

    fn get_or_load_config(&self, file_uri: &Url) -> Option<flask::FlaskConfigHandle> {
        {
            let config = self.read_config();
            if config.is_some() {
                return config.clone();
            }
        }

        let file_path = file_uri.to_file_path().ok()?;
        let file_dir = file_path.parent()?;

        if let Ok(handle) = flask::FlaskConfigHandle::load(file_dir) {
            let mut config = self.write_config();
            *config = Some(handle.clone());
            return Some(handle);
        }

        None
    }

    /// Determine the package root directory for a file URI.
    ///
    /// Searches upward from the file's directory for a `flask.json`. Falls back
    /// to the file's immediate parent directory when no config is found.
    fn package_root_for_uri(&self, uri: &Url) -> Option<std::path::PathBuf> {
        if let Some(handle) = self.get_or_load_config(uri) {
            return Some(handle.source_dir());
        }
        uri.to_file_path()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Collect and publish diagnostics for every `.gin` file in the package
    /// that contains `trigger_file`.
    ///
    /// This mirrors the analysis performed by `begin build`: all files in the
    /// package are parsed, a shared type environment is built, and diagnostics
    /// (parse + type-check + flow-analysis) are collected and published per-file.
    async fn publish_diagnostics_for(&self, uri: Url, trigger_file: File, trigger_source: &str) {
        if self.is_shutdown() {
            return;
        }
        let pkg_root = self.package_root_for_uri(&uri);

        // Discover all `.gin` files in the package directory.
        let all_files: Vec<File> = if let Some(root) = &pkg_root {
            let mut host = self.lock_host();
            host.load_package(root).files
        } else {
            vec![trigger_file]
        };

        let snapshot = self.snapshot();

        // Parse all files and collect parse/lex diagnostics.
        let mut all_asts: Vec<FileAst> = Vec::with_capacity(all_files.len());
        let mut all_span_tables: Vec<_> = Vec::with_capacity(all_files.len());
        let mut all_parse_symptoms: Vec<Vec<_>> = Vec::with_capacity(all_files.len());
        let mut all_sources: Vec<String> = Vec::with_capacity(all_files.len());

        for &f in &all_files {
            if self.is_shutdown() {
                return;
            }
            // Source text: prefer the in-memory document (may have unsaved
            // edits) over the on-disk copy stored in the database.
            let source = if f == trigger_file {
                trigger_source.to_string()
            } else if let Ok(doc_uri) = Url::from_file_path(f.path(&snapshot.db)) {
                if let Some(doc) = self.documents.get(&doc_uri.to_string()) {
                    doc.source.clone()
                } else {
                    f.contents(&snapshot.db).to_string()
                }
            } else {
                f.contents(&snapshot.db).to_string()
            };

            let output = parse_source_full(&source);

            let mut symptoms = Vec::new();
            for &span_id in &output.unterminated_strings {
                symptoms.push(LexSymptom::UnclosedString.into_symptom(span_id));
            }
            for (s, span_id) in &output.lex_errors {
                symptoms.push(s.clone().into_symptom(*span_id));
            }
            for err in &output.parse_errors {
                symptoms.push(ParseSymptom::Custom(err.message.clone()).into_symptom(err.span));
            }
            for (suggested, span_id) in &output.help_hints {
                symptoms.push(
                    ParseSymptom::EmptyParens {
                        suggested: suggested.clone(),
                    }
                    .into_symptom(*span_id),
                );
            }
            for (value, span_id) in &output.unused_values {
                symptoms.push(
                    ParseSymptom::UnusedValue {
                        value: value.clone(),
                    }
                    .into_symptom(*span_id),
                );
            }

            all_asts.push(output.ast);
            all_span_tables.push(output.span_table);
            all_parse_symptoms.push(symptoms);
            all_sources.push(source);
            tokio::task::yield_now().await;
        }

        // Run type-check and flow analysis, then publish diagnostics per file.
        for (i, &pkg_file) in all_files.iter().enumerate() {
            if self.is_shutdown() {
                return;
            }
            let pkg_path = pkg_file.path(&snapshot.db);
            let pkg_uri = match Url::from_file_path(&pkg_path) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let mut symptoms = all_parse_symptoms[i].clone();

            // Type-check and flow-analysis symptoms
            symptoms.extend(analyze_file(&all_asts[i], &all_asts));

            // Cache a clean AST for the trigger file so that other handlers
            // (goto-definition, hover, completion) can use it.
            if pkg_file == trigger_file && symptoms.is_empty() {
                self.ast_cache
                    .insert(uri.to_string(), Arc::new(all_asts[i].clone()));
            }

            // Wrap symptoms for compatibility with symptoms_to_diagnostics
            let wrapped: Vec<Symptoms> = symptoms.into_iter().map(Symptoms).collect();
            let symptom_refs: Vec<&Symptoms> = wrapped.iter().collect();

            let diagnostics =
                symptoms_to_diagnostics(&all_sources[i], &all_span_tables[i], &symptom_refs);
            self.client
                .publish_diagnostics(pkg_uri, diagnostics, None)
                .await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.catch_request("initialize", self.handle_initialize(params))
            .await
    }

    async fn initialized(&self, params: InitializedParams) {
        self.catch_notification("initialized", self.handle_initialized(params))
            .await
    }

    async fn shutdown(&self) -> Result<()> {
        self.catch_request("shutdown", self.handle_shutdown()).await
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.catch_notification("did_open", self.handle_did_open(params))
            .await
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.catch_notification("did_change", self.handle_did_change(params))
            .await
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.catch_notification("did_save", self.handle_did_save(params))
            .await
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.catch_notification("did_close", self.handle_did_close(params))
            .await
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.catch_request("goto_definition", self.handle_goto_definition(params))
            .await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.catch_request("references", self.handle_references(params))
            .await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.catch_request("completion", self.handle_completion(params))
            .await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.catch_request("hover", self.handle_hover(params)).await
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        self.catch_request("signature_help", self.handle_signature_help(params))
            .await
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        self.catch_request("formatting", self.handle_formatting(params))
            .await
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use internment::Intern;
    use parser::parse_from_str;
    use typeck::TyEnv;

    #[test]
    fn dot_completion_union_variants() {
        use crate::handlers::completion::dot_completions;

        let source = "Maybe(x) is Some(x) or None";
        let ast = parse_from_str(source);
        let ty_env = TyEnv::from_file_ast(&ast);

        let ty = ty_env
            .resolve_dot_type(&ast, Intern::<String>::from_ref("Maybe"))
            .expect("Expected Maybe to resolve to a union type");
        let items = dot_completions(ty);

        assert_eq!(items.len(), 2, "Expected 2 variants for Maybe type");

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Some(x)"), "Expected 'Some(x)' variant");
        assert!(labels.contains(&"None"), "Expected 'None' variant");

        let some_item = items.iter().find(|i| i.label == "Some(x)").unwrap();
        assert_eq!(some_item.detail.as_ref().unwrap(), &"Maybe.Some(x)");
    }
}
