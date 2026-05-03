mod diagnostics;
mod handlers;
mod state;

use dashmap::DashMap;
use database::Diagnostics;
use database::File;
use database::{file_parse_output, package_typecheck_symptoms, sorted_package_files, PackageFiles};
use diagnostics::symptoms_to_diagnostics;
use futures::FutureExt;
use state::{DocumentState, GinHost, JsonDocumentState};
use std::ops::Deref;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// Final safety net for a single package-wide diagnostic computation. Salsa
/// cancellation already aborts in-flight work as soon as a new edit arrives
/// (see [`Backend::run_blocking_request`] for the read-request equivalent),
/// and the parser is now structurally guaranteed to terminate. This timeout
/// only catches truly pathological compute that the type-checker hasn't been
/// audited for yet — it should virtually never fire in practice.
const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(10);

/// Final safety net for a single read request. Edit-during-compute is handled
/// by Salsa cancellation, so this only fires on type-checker pathologies.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Canonical directory containing `flask.jsonc` for `from_dir`, if any (walks upward like
/// [`flask::FlaskConfigHandle::load`]).
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

/// Shared LSP state. Heavy work (package diagnostics) runs on the blocking
/// pool so panicking/spinning parsers cannot starve the async runtime, and
/// stale results are filtered via a monotonic generation counter rather than
/// `JoinHandle::abort` (which is a no-op for sync CPU loops).
pub(crate) struct GinLspBackend {
    pub(crate) client: Client,
    pub(crate) host: Arc<Mutex<GinHost>>,
    pub(crate) documents: DashMap<String, DocumentState>,
    pub(crate) json_documents: DashMap<String, JsonDocumentState>,
    /// One cached [`flask::FlaskConfigHandle`] per package root (`flask.jsonc` directory).
    /// Unlike a single global handle, this stays correct in multi-package workspaces.
    pub(crate) package_configs: DashMap<std::path::PathBuf, flask::FlaskConfigHandle>,
    pub(crate) shutdown: AtomicBool,
    /// Bumped on every `spawn_publish_diagnostics`. A worker that observes a
    /// newer value than the one it was spawned with discards its results.
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

    /// Bump the diagnostic generation and kick off a worker for it. The
    /// caller must have already written the new file contents to the host
    /// database (via [`state::GinHost::upsert_file`] in the document-sync
    /// handlers); this is what arms Salsa cancellation against any earlier
    /// in-flight worker.
    pub(crate) fn spawn_publish_diagnostics(&self, uri: Url, file: File) {
        let my_gen = self
            .latest_diagnostic_gen
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let this = self.clone();
        tokio::spawn(async move {
            this.publish_diagnostics_for(uri, file, my_gen).await;
        });
    }

    fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// True if a newer `spawn_publish_diagnostics` has been issued since `my_gen`.
    fn is_stale(&self, my_gen: u64) -> bool {
        self.latest_diagnostic_gen.load(Ordering::SeqCst) != my_gen
    }

    /// Run a Salsa-touching request on the blocking pool with cooperative
    /// cancellation. Returns `None` on shutdown, cancellation, panic, or
    /// timeout — the LSP handler maps that to "no result" so the editor's
    /// spinner clears.
    ///
    /// Cancellation flow: every Salsa setter call (e.g. `set_file_contents`
    /// from `did_change`) bumps the database revision. In-flight queries on
    /// snapshot clones detect the bumped revision at their next query
    /// boundary and panic with [`salsa::Cancelled::PendingWrite`]. We catch
    /// that here as a clean `Err` so the worker thread returns immediately
    /// instead of running stale-and-doomed work to completion. This is the
    /// idiomatic Salsa pattern and the reason rust-analyzer stays snappy on
    /// large crates while you type.
    ///
    /// The wall-clock [`REQUEST_TIMEOUT`] is a final safety net for the case
    /// where compute is slow but no edit has arrived to cancel it (e.g. an
    /// unbounded type-checker recursion); it should virtually never fire.
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
            // `Backend` contains `Mutex` / `RwLock` which poison on panic;
            // catching `salsa::Cancelled` here cannot leave shared state in
            // an unsafe limbo, so `AssertUnwindSafe` is sound. Same pattern
            // as the existing `Backend::catch_request` panic handler.
            // `Err(salsa::Cancelled)` means a new edit invalidated our
            // snapshot — normal operation, not an error. The next request
            // will be served from the fresher revision.
            let work = AssertUnwindSafe(move || f(this));
            salsa::Cancelled::catch(work).ok()
        });
        match tokio::time::timeout(REQUEST_TIMEOUT, blocking).await {
            Ok(Ok(value)) => value,
            Ok(Err(join_err)) => {
                // Non-cancellation panic (real bug). Cancellations are
                // converted to `None` inside the closure above.
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
            eprintln!("[ginlsp] notification handler '{}' panicked: {}", name, msg);
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

    /// Determine the package root directory for a file URI.
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
    /// Cancellation: the worker runs on the blocking pool inside
    /// [`salsa::Cancelled::catch`]. When a fresh `did_change` arrives during
    /// compute, that handler's `set_file_contents` call bumps the database
    /// revision and our worker's next Salsa query unwinds via Cancelled. We
    /// observe an empty payload and publish nothing — the new edit will spawn
    /// its own worker for the fresher state. [`DIAGNOSTIC_TIMEOUT`] only
    /// triggers when compute is slow but no edit arrived to cancel it (e.g.
    /// type-checker pathologies); in steady state it should never fire.
    async fn publish_diagnostics_for(&self, uri: Url, trigger_file: File, my_gen: u64) {
        if self.is_shutdown() || self.is_stale(my_gen) {
            return;
        }
        let pkg_root = self.package_root_for_uri(&uri);

        let this = self.clone();
        let blocking = tokio::task::spawn_blocking(move || -> Vec<(Url, Vec<Diagnostic>)> {
            // `Err(salsa::Cancelled)` → empty payload → publish nothing; the
            // newer worker that triggered the cancellation will publish.
            let work =
                AssertUnwindSafe(move || this.compute_package_diagnostics(pkg_root, trigger_file));
            salsa::Cancelled::catch(work).unwrap_or_default()
        });

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

    /// Synchronous half of [`publish_diagnostics_for`]: discover package files,
    /// run parse + typecheck on a Salsa snapshot, and assemble per-file
    /// diagnostics. Cancellation is delivered by Salsa unwinding through this
    /// function — the caller [`salsa::Cancelled::catch`]es it.
    ///
    /// Note: the trigger file's contents were already written to the host
    /// database in `handle_did_change` before this worker spawned; we do not
    /// re-write them here, both to avoid a redundant revision bump and to
    /// ensure this worker's snapshot is consistent with the edit that
    /// scheduled it.
    fn compute_package_diagnostics(
        &self,
        pkg_root: Option<std::path::PathBuf>,
        trigger_file: File,
    ) -> Vec<(Url, Vec<Diagnostic>)> {
        if self.is_shutdown() {
            return Vec::new();
        }

        let all_files: Vec<File> = if let Some(root) = &pkg_root {
            let mut host = self.lock_host();
            host.load_package(root).files
        } else {
            vec![trigger_file]
        };

        let snapshot = self.snapshot();

        if self.is_shutdown() {
            return Vec::new();
        }

        let package_files = sorted_package_files(&snapshot.db, &all_files);
        let pkg = PackageFiles::new(&snapshot.db, package_files.clone());
        let typecheck_symptoms = package_typecheck_symptoms(&snapshot.db, pkg);

        if self.is_shutdown() {
            return Vec::new();
        }

        let mut results = Vec::with_capacity(package_files.len());
        for (i, &pkg_file) in package_files.iter().enumerate() {
            let pkg_path = pkg_file.path(&snapshot.db);
            let pkg_uri = match Url::from_file_path(&pkg_path) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let parse = file_parse_output(&snapshot.db, pkg_file);
            let source = pkg_file.contents(&snapshot.db).to_string();
            let mut symptoms = parse.symptoms.clone();
            symptoms.extend(typecheck_symptoms[i].iter().cloned());

            let wrapped: Vec<Diagnostics> = symptoms.into_iter().map(Diagnostics).collect();
            let symptom_refs: Vec<&Diagnostics> = wrapped.iter().collect();

            let diagnostics =
                symptoms_to_diagnostics(&source, parse.ast.span_table(), &symptom_refs, &pkg_uri);
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
    use typeck::TyEnv;

    #[test]
    fn dot_completion_union_variants() {
        use crate::handlers::completion::dot_completions;

        let source = "Maybe[x] is Some(x) or None";
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
