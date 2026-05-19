#![deny(unsafe_code)]
#![warn(
    clippy::correctness,
    clippy::suspicious,
    clippy::style,
    clippy::complexity,
    clippy::perf
)]
mod diagnostics;
mod handlers;
mod state;

use dashmap::DashMap;

use ast::HasSpanId;
use ast::ImportSource;
use diagnostics::symptoms_to_diagnostics;
use futures::FutureExt;
use resolve::{resolve_flask_path_dependencies, ParsedFile};
use state::{DocumentState, GinHost, JsonDocumentState};
use std::collections::HashMap;
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Produce LSP diagnostics and code-action data for `use` statements whose
/// comma-separated items are not in alphabetical order.
fn find_unsorted_imports(source: &str, file_ast: &ast::FileAst) -> Vec<Diagnostic> {
    fn import_sort_key(mi: &ast::ModuleImport) -> String {
        match &mi.source {
            ImportSource::Package(path) => {
                let mut s = path.root.as_str().to_lowercase();
                for seg in &path.segments {
                    s.push('.');
                    s.push_str(seg.as_str().to_lowercase().as_str());
                }
                s
            }
            ImportSource::Local(p, _) => p.to_string_lossy().to_lowercase(),
            ImportSource::LocalBundle(b) => b.root.as_str().to_lowercase(),
            ImportSource::CurrentModule { member } => member.export.as_str().to_lowercase(),
        }
    }

    fn is_unsorted(items: &[&ast::ModuleImport]) -> bool {
        items
            .windows(2)
            .any(|w| import_sort_key(w[0]) > import_sort_key(w[1]))
    }

    /// Compute the byte span of a full `use …` line from the first and last
    /// `ModuleImport` source spans.
    fn use_line_span(
        first_span_id: ast::span::SpanId,
        last_span_id: ast::span::SpanId,
        source: &str,
        span_table: &ast::SpanTable,
    ) -> Option<(usize, usize)> {
        let first = span_table.get(first_span_id);
        let last = span_table.get(last_span_id);
        // Walk backward from first bytes to find "use "
        let line_start = source[..first.start]
            .rfind('\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        // Walk forward from last bytes to end of line
        let line_end = source[last.end..]
            .find('\n')
            .map(|p| last.end + p)
            .unwrap_or(source.len());
        Some((line_start, line_end))
    }

    let span_table = file_ast.span_table();
    let mut result = Vec::new();

    for import in &file_ast.uses {
        if import.0.len() < 2 {
            continue;
        }
        let items: Vec<&ast::ModuleImport> = import.0.iter().collect();
        if !is_unsorted(&items) {
            continue;
        }

        let first_id = match items.first() {
            Some(mi) => mi.source.span_id(),
            None => continue,
        };
        let last_id = match items.last() {
            Some(mi) => mi.source.span_id(),
            None => continue,
        };

        let (line_start_byte, line_end_byte) =
            match use_line_span(first_id, last_id, source, span_table) {
                Some(spans) => spans,
                None => continue,
            };

        // Build the sorted version
        let mut sorted: Vec<&ast::ModuleImport> = items.clone();
        sorted.sort_by_key(|mi| import_sort_key(mi));

        let mut sorted_text = String::from("use ");
        for (i, mi) in sorted.iter().enumerate() {
            if i > 0 {
                sorted_text.push_str(", ");
            }
            match &mi.source {
                ImportSource::Package(path) => {
                    sorted_text.push_str(path.root.as_str());
                    for seg in &path.segments {
                        sorted_text.push('.');
                        sorted_text.push_str(seg.as_str());
                    }
                }
                ImportSource::Local(p, _) => {
                    sorted_text.push('\'');
                    sorted_text.push_str(&p.to_string_lossy());
                    sorted_text.push('\'');
                }
                ImportSource::LocalBundle(b) => {
                    sorted_text.push_str(b.root.as_str());
                    sorted_text.push_str(".(");
                    let mut members: Vec<&ast::BundleExportImport> = b.members.iter().collect();
                    members.sort_by_key(|m| m.export.as_str().to_lowercase());
                    for (j, member) in members.iter().enumerate() {
                        if j > 0 {
                            sorted_text.push_str(", ");
                        }
                        sorted_text.push_str(member.export.as_str());
                        if let Some(alias) = &member.alias {
                            sorted_text.push_str(" as ");
                            sorted_text.push_str(alias.as_str());
                        }
                    }
                    sorted_text.push(')');
                }
                ImportSource::CurrentModule { member } => {
                    sorted_text.push_str(member.export.as_str());
                    if let Some(alias) = &member.alias {
                        sorted_text.push_str(" as ");
                        sorted_text.push_str(alias.as_str());
                    }
                }
            }
        }

        use ast::byte_offset_to_position;
        let (sl, sc) = byte_offset_to_position(line_start_byte, source);
        let (el, ec) = byte_offset_to_position(line_end_byte, source);

        let range = Range {
            start: Position {
                line: sl,
                character: sc,
            },
            end: Position {
                line: el,
                character: ec,
            },
        };

        result.push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::HINT),
            code: Some(NumberOrString::String("ginfmt.sort-imports".to_string())),
            code_description: None,
            source: Some("ginfmt".to_string()),
            message: "Imports are not in alphabetical order".to_string(),
            related_information: None,
            tags: None,
            data: Some(serde_json::json!({
                "gincQuickFix": "sort-imports",
                "sortedText": sorted_text,
            })),
        });
    }

    result
}

fn package_root_containing(from_dir: &Path) -> Option<std::path::PathBuf> {
    let mut search = from_dir.to_path_buf();
    loop {
        search.push(flask::PACKAGE_CONFIG_NAME);
        if search.exists() {
            search.pop();
            return std::fs::canonicalize(&search).ok().or(Some(search));
        }
        search.pop();
        if !search.pop() {
            return None;
        }
    }
}

pub(crate) struct GinLspBackend {
    pub(crate) client: Client,
    pub(crate) host: Arc<Mutex<GinHost>>,
    pub(crate) documents: DashMap<String, DocumentState>,
    pub(crate) json_documents: DashMap<String, JsonDocumentState>,
    pub(crate) package_configs: DashMap<std::path::PathBuf, flask::FlaskConfigHandle>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) latest_diagnostic_gen: AtomicU64,
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
            package_configs: DashMap::new(),
            shutdown: AtomicBool::new(false),
            latest_diagnostic_gen: AtomicU64::new(0),
        }))
    }

    pub(crate) fn spawn_publish_diagnostics(&self, uri: Url, file_path: PathBuf) {
        let my_gen = self
            .latest_diagnostic_gen
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let this = self.clone();
        tokio::spawn(async move {
            this.publish_diagnostics_for(uri, file_path, my_gen).await;
        });
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    fn is_stale(&self, my_gen: u64) -> bool {
        self.latest_diagnostic_gen.load(Ordering::SeqCst) != my_gen
    }

    async fn run_blocking_request<F, T>(&self, name: &'static str, f: F) -> Option<T>
    where
        F: FnOnce(Backend) -> T + Send + 'static,
        T: Send + 'static,
    {
        if self.is_shutdown() {
            return None;
        }
        let this = self.clone();
        let blocking = tokio::task::spawn_blocking(move || -> Option<T> {
            let work = AssertUnwindSafe(move || f(this));
            salsa::Cancelled::catch(work).ok()
        });
        match tokio::time::timeout(REQUEST_TIMEOUT, blocking).await {
            Ok(Ok(value)) => value,
            Ok(Err(join_err)) => {
                eprintln!("[ginlsp] '{name}' worker panicked: {join_err}");
                None
            }
            Err(_) => {
                eprintln!(
                    "[ginlsp] '{name}' exceeded {:?}; abandoning",
                    REQUEST_TIMEOUT
                );
                None
            }
        }
    }

    fn lock_host(&self) -> std::sync::MutexGuard<'_, GinHost> {
        self.host.lock().unwrap_or_else(|e| e.into_inner())
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
            eprintln!("[ginlsp] notification '{}' panicked: {}", name, msg);
        }
    }

    fn snapshot(&self) -> state::GinSnapshot {
        let host = self.lock_host();
        host.snapshot()
    }

    fn get_or_load_config(&self, file_uri: &Url) -> Option<flask::FlaskConfigHandle> {
        let file_path = file_uri.to_file_path().ok()?;
        let file_dir = file_path.parent()?;
        let cache_key = package_root_containing(file_dir)?;
        if let Some(existing) = self.package_configs.get(&cache_key) {
            return Some(existing.clone());
        }
        let loaded = flask::FlaskConfigHandle::load(file_dir).ok()?;
        self.package_configs.insert(cache_key, loaded.clone());
        Some(loaded)
    }

    fn package_root_for_uri(&self, uri: &Url) -> Option<std::path::PathBuf> {
        if let Some(handle) = self.get_or_load_config(uri) {
            return Some(handle.source_dir());
        }
        uri.to_file_path()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    async fn publish_diagnostics_for(&self, uri: Url, trigger_path: PathBuf, my_gen: u64) {
        if self.is_shutdown() || self.is_stale(my_gen) {
            return;
        }
        let config_handle = self.get_or_load_config(&uri);
        let pkg_root = config_handle
            .as_ref()
            .map(|handle| handle.source_dir())
            .or_else(|| {
                uri.to_file_path()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            });

        let this = self.clone();
        let blocking = tokio::task::spawn_blocking(
            move || -> Vec<(Url, Vec<tower_lsp::lsp_types::Diagnostic>)> {
                let work = AssertUnwindSafe(move || {
                    this.compute_package_diagnostics(pkg_root, config_handle.clone(), trigger_path)
                });
                salsa::Cancelled::catch(work).unwrap_or_default()
            },
        );

        let payload = match tokio::time::timeout(DIAGNOSTIC_TIMEOUT, blocking).await {
            Ok(Ok(payload)) => payload,
            Ok(Err(join_err)) => {
                eprintln!("[ginlsp] diagnostic worker panicked: {join_err}");
                return;
            }
            Err(_) => {
                eprintln!(
                    "[ginlsp] diagnostic computation exceeded {:?}; abandoning gen {}",
                    DIAGNOSTIC_TIMEOUT, my_gen
                );
                return;
            }
        };

        for (pkg_uri, diags) in payload {
            if self.is_shutdown() || self.is_stale(my_gen) {
                return;
            }
            self.client.publish_diagnostics(pkg_uri, diags, None).await;
        }
    }

    fn compute_package_diagnostics(
        &self,
        pkg_root: Option<PathBuf>,
        config_handle: Option<flask::FlaskConfigHandle>,
        trigger_path: PathBuf,
    ) -> Vec<(Url, Vec<tower_lsp::lsp_types::Diagnostic>)> {
        if self.is_shutdown() {
            return Vec::new();
        }

        let dependency_dirs = config_handle
            .as_ref()
            .map(|h| {
                let cfg = h.read();
                resolve_flask_path_dependencies(&cfg.config, &h.source_dir())
            })
            .unwrap_or_default();

        let all_file_paths: Vec<PathBuf> = if let Some(root) = &pkg_root {
            let mut host = self.lock_host();
            host.load_package(root).file_paths
        } else {
            vec![trigger_path.clone()]
        };

        let snapshot = self.snapshot();

        if self.is_shutdown() {
            return Vec::new();
        }

        let typecheck_symptoms = snapshot.engine.typecheck_package(&all_file_paths);

        if self.is_shutdown() {
            return Vec::new();
        }

        let import_symptoms_by_path: HashMap<PathBuf, Vec<diagnostic::Diagnostic>> =
            if !dependency_dirs.is_empty() {
                let entry_files: Vec<ParsedFile> = all_file_paths
                    .iter()
                    .filter_map(|path| {
                        let (source, parse) = snapshot.engine.source_and_parse(path)?;
                        Some(ParsedFile {
                            path: path.clone(),
                            source,
                            output: (*parse).clone(),
                        })
                    })
                    .collect();

                resolve::resolve_import_symptoms(entry_files, &dependency_dirs)
            } else {
                HashMap::new()
            };

        let mut results = Vec::with_capacity(all_file_paths.len());
        for (i, pkg_path) in all_file_paths.iter().enumerate() {
            let pkg_uri = match Url::from_file_path(pkg_path) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let mut symptoms = Vec::new();
            if let Some(diags) = import_symptoms_by_path.get(pkg_path) {
                symptoms.extend(diags.iter().cloned());
            }
            if i < typecheck_symptoms.len() {
                symptoms.extend(typecheck_symptoms[i].iter().cloned());
            }

            let source_and_span = snapshot
                .engine
                .source_and_parse(pkg_path)
                .map(|(s, po)| (s, po.ast.span_table().clone()));

            let parse_output = snapshot.engine.source_and_parse(pkg_path);

            let mut diagnostics = match source_and_span {
                Some((ref source, ref span_table)) => {
                    symptoms_to_diagnostics(source, span_table, &symptoms, &pkg_uri)
                }
                None => Vec::new(),
            };

            // Check for unsorted imports and emit hints
            if let Some((ref source, ref _po)) = parse_output {
                let unsorted = find_unsorted_imports(source, &_po.ast);
                diagnostics.extend(unsorted);
            }

            results.push((pkg_uri, diagnostics));
        }
        results
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

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        self.catch_request("code_action", self.handle_code_action(params))
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

    #[test]
    fn dot_completion_union_variants() {
        use crate::handlers::completion::dot_completions;
        use ast::typed::transform_file;

        let source = "Maybe(x) is Some(x) or None";
        let po = parse_from_str(source);
        let typed = transform_file(po.clone(), ast::typed::FileId(0));

        let ty = typed
            .tag_types
            .get(&ast::typed::TagId(Intern::<String>::from_ref("Maybe")))
            .cloned()
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
