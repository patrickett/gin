mod diagnostics;
mod handlers;
mod state;

use dashmap::DashMap;
use diagnostics::symptoms_to_diagnostics;
use ginc::FileAst;
use state::{DocumentState, GinHost, JsonDocumentState};
use std::sync::{Arc, Mutex, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

struct Backend {
    client: Client,
    host: Arc<Mutex<GinHost>>,
    documents: DashMap<String, DocumentState>,
    json_documents: DashMap<String, JsonDocumentState>,
    config: RwLock<Option<flask::FlaskConfigHandle>>,
    ast_cache: DashMap<String, Arc<FileAst>>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Self {
            client,
            host: Arc::new(Mutex::new(GinHost::new())),
            documents: DashMap::new(),
            json_documents: DashMap::new(),
            config: RwLock::new(None),
            ast_cache: DashMap::new(),
        }
    }

    fn snapshot(&self) -> state::GinSnapshot {
        let host = self.host.lock().unwrap();
        host.snapshot()
    }

    fn get_or_load_config(&self, file_uri: &Url) -> Option<flask::FlaskConfigHandle> {
        {
            let config = self.config.read().unwrap();
            if config.is_some() {
                return config.clone();
            }
        }

        let file_path = file_uri.to_file_path().ok()?;
        let file_dir = file_path.parent()?;

        if let Ok(handle) = flask::FlaskConfigHandle::load(file_dir) {
            let mut config = self.config.write().unwrap();
            *config = Some(handle.clone());
            return Some(handle);
        }

        None
    }

    async fn publish_diagnostics_for(&self, uri: Url, file: ginc::File, source: &str) {
        let snapshot = self.snapshot();
        let ast = snapshot.parse(file);
        let symptoms = snapshot.diagnostics(file);
        if symptoms.is_empty() {
            self.ast_cache
                .insert(uri.to_string(), Arc::new(ast));
        }
        let diagnostics = symptoms_to_diagnostics(source, &symptoms[..]);
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.handle_initialize(params).await
    }

    async fn initialized(&self, params: InitializedParams) {
        self.handle_initialized(params).await
    }

    async fn shutdown(&self) -> Result<()> {
        self.handle_shutdown().await
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.handle_did_open(params).await
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.handle_did_change(params).await
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.handle_did_save(params).await
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.handle_did_close(params).await
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        self.handle_goto_definition(params).await
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        self.handle_references(params).await
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        self.handle_completion(params).await
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        self.handle_hover(params).await
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        self.handle_signature_help(params).await
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        self.handle_formatting(params).await
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
    use ginc::parse::parse_from_str;
    use ginc::typeck::TyEnv;

    #[test]
    fn dot_completion_union_variants() {
        use crate::handlers::completion::dot_completions;
        use ginc::intern::IStr;

        let source = "Maybe(x) is Some(x) or None";
        let ast = parse_from_str(source);
        let ty_env = TyEnv::from_file_ast(&ast);

        let ty = ty_env
            .resolve_dot_type(&ast, IStr::new("Maybe".to_string()))
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
