//! The protocol loop and the state of open documents.
//!
//! Single-threaded and synchronous. Diagnostics are computed when no further input is already
//! buffered, so a burst of edits is checked once, on the last of them. Every request is answered
//! exactly once; a malformed message gets an error response and never ends the server.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, HoverParams, HoverProviderCapability, InitializeParams,
    InitializeResult, OneOf, PublishDiagnosticsParams, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url, WorkspaceSymbolParams,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::analysis::{Unit, analyze, canonical, package_root};
use crate::diagnostics::convert;
use crate::hover::{HoverState, hover};
use crate::navigate::{definition, workspace_symbols};
use crate::position::LineIndex;
use crate::symbols::document_symbols;
use crate::transport::{ReadError, read_message, write_message};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const SERVER_NOT_INITIALIZED: i64 = -32002;
pub const REQUEST_FAILED: i64 = -32803;

/// More open documents than this are refused (their open is ignored).
const DOCUMENTS_MAX: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Uninitialized,
    Running,
    ShuttingDown,
}

struct Doc {
    version: i32,
    text: String,
    index: LineIndex,
    path: PathBuf,
    hover: Option<HoverState>,
}

impl Doc {
    fn new(uri: &Url, version: i32, text: String) -> Self {
        let path = uri
            .to_file_path()
            .unwrap_or_else(|()| PathBuf::from(uri.path()));
        Self {
            version,
            index: LineIndex::new(&text),
            text,
            path,
            hover: None,
        }
    }

    fn replace(&mut self, text: String) {
        self.index = LineIndex::new(&text);
        self.text = text;
        self.hover = None;
    }
}

/// A response or notification waiting to be written.
type Outbox = Vec<Value>;

fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn result_response(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

pub struct Server {
    phase: Phase,
    docs: BTreeMap<Url, Doc>,
    roots: Vec<PathBuf>,
    dirty: BTreeSet<Url>,
    /// Last complete diagnostic batch per URI, including the open document's version.
    published: BTreeMap<Url, (Option<i32>, Value)>,
    exit_code: Option<i32>,
    /// Closed documents whose diagnostics are to be cleared.
    pending_clear: Vec<Url>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            phase: Phase::Uninitialized,
            docs: BTreeMap::new(),
            roots: Vec::new(),
            dirty: BTreeSet::new(),
            published: BTreeMap::new(),
            exit_code: None,
            pending_clear: Vec::new(),
        }
    }
}

fn params<T: DeserializeOwned>(message: &Value) -> Result<T, String> {
    serde_json::from_value(message.get("params").cloned().unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

impl Server {
    /// Handle one decoded message, returning what to send.
    pub fn handle(&mut self, message: &Value) -> Outbox {
        let mut out = Outbox::new();
        let Some(object) = message.as_object() else {
            out.push(error_response(
                &Value::Null,
                INVALID_REQUEST,
                "a message must be a JSON object",
            ));
            return out;
        };
        let id = object.get("id");
        if let Some(id) = id {
            if !(id.is_string() || id.is_i64() || id.is_u64())
                && !(object.contains_key("result") || object.contains_key("error"))
            {
                out.push(error_response(
                    &Value::Null,
                    INVALID_REQUEST,
                    "id must be a string or an integer",
                ));
                return out;
            }
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            // A response to a request of ours is ignored; anything else is not a message.
            if !(object.contains_key("result") || object.contains_key("error")) || id.is_none() {
                out.push(error_response(
                    id.unwrap_or(&Value::Null),
                    INVALID_REQUEST,
                    "a message needs a string `method`",
                ));
            }
            return out;
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.dispatch(method, id, message, &mut out)
        }));
        if result.is_err() {
            if let Some(id) = id {
                out.push(error_response(
                    id,
                    INTERNAL_ERROR,
                    &format!("internal error handling {method}"),
                ));
            }
        }
        out
    }

    fn dispatch(&mut self, method: &str, id: Option<&Value>, message: &Value, out: &mut Outbox) {
        if method == "exit" {
            self.exit_code = Some(if self.phase == Phase::ShuttingDown {
                0
            } else {
                1
            });
            return;
        }
        // Requests carry an id; notifications do not and are never answered.
        let Some(id) = id else {
            if self.phase == Phase::Running {
                self.notify(method, message);
            }
            return;
        };
        match (self.phase, method) {
            (Phase::Uninitialized, "initialize") => match params::<InitializeParams>(message) {
                Ok(p) => {
                    self.roots = roots_of(&p);
                    self.phase = Phase::Running;
                    out.push(result_response(id, initialize_result()));
                }
                Err(e) => out.push(error_response(
                    id,
                    INVALID_PARAMS,
                    &format!("invalid initialize params: {e}"),
                )),
            },
            (Phase::Uninitialized, _) => out.push(error_response(
                id,
                SERVER_NOT_INITIALIZED,
                "the server is not initialized",
            )),
            (Phase::ShuttingDown, _) => out.push(error_response(
                id,
                INVALID_REQUEST,
                "the server is shutting down",
            )),
            (Phase::Running, "initialize") => out.push(error_response(
                id,
                INVALID_REQUEST,
                "the server is already initialized",
            )),
            (Phase::Running, "shutdown") => {
                self.phase = Phase::ShuttingDown;
                out.push(result_response(id, Value::Null));
            }
            (Phase::Running, _) => {
                let reply = self.request(method, message);
                out.push(match reply {
                    Ok(result) => result_response(id, result),
                    Err((code, text)) => error_response(id, code, &text),
                });
            }
        }
    }

    fn notify(&mut self, method: &str, message: &Value) {
        match method {
            "textDocument/didOpen" => {
                if let Ok(p) = params::<DidOpenTextDocumentParams>(message) {
                    if !p.text_document.language_id.eq_ignore_ascii_case("gin")
                        || p.text_document.uri.path().ends_with("/mod.flask")
                    {
                        return;
                    }
                    if self.docs.len() < DOCUMENTS_MAX
                        || self.docs.contains_key(&p.text_document.uri)
                    {
                        let doc = Doc::new(
                            &p.text_document.uri,
                            p.text_document.version,
                            p.text_document.text,
                        );
                        self.docs.insert(p.text_document.uri, doc);
                        self.invalidate_documents();
                    }
                }
            }
            "textDocument/didChange" => {
                if let Ok(p) = params::<DidChangeTextDocumentParams>(message) {
                    if let Some(doc) = self.docs.get_mut(&p.text_document.uri) {
                        let mut text = doc.text.clone();
                        for change in p.content_changes {
                            match change.range {
                                None => text = change.text,
                                Some(range) => {
                                    let index = LineIndex::new(&text);
                                    let (start, end) = (
                                        index.offset(&text, range.start),
                                        index.offset(&text, range.end),
                                    );
                                    text.replace_range(
                                        start.min(end)..end.max(start),
                                        &change.text,
                                    );
                                }
                            }
                        }
                        doc.version = p.text_document.version;
                        doc.replace(text);
                        self.invalidate_documents();
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(uri) = message
                    .pointer("/params/textDocument/uri")
                    .and_then(Value::as_str)
                    .and_then(|u| Url::parse(u).ok())
                {
                    if self.docs.contains_key(&uri) {
                        self.invalidate_documents();
                    }
                }
            }
            "textDocument/didClose" => {
                if let Ok(p) = params::<DidCloseTextDocumentParams>(message) {
                    self.docs.remove(&p.text_document.uri);
                    self.dirty.remove(&p.text_document.uri);
                    self.invalidate_documents();
                    self.pending_clear.push(p.text_document.uri);
                }
            }
            _ => {}
        }
    }

    fn invalidate_documents(&mut self) {
        assert!(self.docs.len() <= DOCUMENTS_MAX);
        for doc in self.docs.values_mut() {
            doc.hover = None;
        }
        self.dirty.extend(self.docs.keys().cloned());
        assert_eq!(self.dirty.len(), self.docs.len());
    }

    fn request(&mut self, method: &str, message: &Value) -> Result<Value, (i64, String)> {
        let invalid = |e: String| (INVALID_PARAMS, format!("invalid params for {method}: {e}"));
        match method {
            "textDocument/hover" => {
                let p: HoverParams = params(message).map_err(invalid)?;
                let uri = p.text_document_position_params.text_document.uri;
                let position = p.text_document_position_params.position;
                let Some(doc) = self.docs.get_mut(&uri) else {
                    return Ok(Value::Null);
                };
                let offset = doc.index.offset(&doc.text, position);
                let file = doc.path.display().to_string();
                let state = doc
                    .hover
                    .get_or_insert_with(|| HoverState::new(&file, &doc.text));
                Ok(hover(state, &doc.text, &doc.index, offset).map_or(Value::Null, |h| json!(h)))
            }
            "textDocument/documentSymbol" => {
                let p: DocumentSymbolParams = params(message).map_err(invalid)?;
                let Some(doc) = self.docs.get(&p.text_document.uri) else {
                    return Ok(Value::Null);
                };
                let Ok(output) = parser::parse(&doc.text) else {
                    return Ok(json!([]));
                };
                Ok(json!(DocumentSymbolResponse::Nested(document_symbols(
                    &output.file,
                    &doc.text,
                    &doc.index
                ))))
            }
            "textDocument/formatting" => {
                let p: DocumentFormattingParams = params(message).map_err(invalid)?;
                let Some(doc) = self.docs.get(&p.text_document.uri) else {
                    return Ok(Value::Null);
                };
                let formatted = ginfmt::format(&doc.text).map_err(|error| {
                    let code = match &error {
                        ginfmt::FormatError::Flaws(_) => REQUEST_FAILED,
                        ginfmt::FormatError::Unverified(_) => INTERNAL_ERROR,
                    };
                    (code, format!("cannot format source: {error}"))
                })?;
                if formatted == doc.text {
                    return Ok(json!([]));
                }
                let end = doc.index.position(&doc.text, doc.text.len());
                Ok(json!([lsp_types::TextEdit {
                    range: lsp_types::Range {
                        start: lsp_types::Position::new(0, 0),
                        end
                    },
                    new_text: formatted,
                }]))
            }
            "textDocument/definition" => {
                let p: GotoDefinitionParams = params(message).map_err(invalid)?;
                let uri = p.text_document_position_params.text_document.uri;
                let Some(doc) = self.docs.get(&uri) else {
                    return Ok(Value::Null);
                };
                let offset = doc
                    .index
                    .offset(&doc.text, p.text_document_position_params.position);
                let read = self.reader();
                Ok(definition(&doc.path, &doc.text, offset, &read)
                    .map_or(Value::Null, |l| json!(GotoDefinitionResponse::Scalar(l))))
            }
            "workspace/symbol" => {
                let p: WorkspaceSymbolParams = params(message).map_err(invalid)?;
                let read = self.reader();
                Ok(json!(workspace_symbols(&self.roots, &p.query, &read)))
            }
            _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
        }
    }

    /// File contents by path: an open buffer first, then the disk.
    fn reader(&self) -> impl Fn(&Path) -> Option<String> {
        let buffers = self.buffers();
        move |path: &Path| {
            buffers
                .get(&canonical(path))
                .cloned()
                .or_else(|| std::fs::read_to_string(path).ok())
        }
    }

    fn buffers(&self) -> BTreeMap<PathBuf, String> {
        self.docs
            .values()
            .map(|d| (canonical(&d.path), d.text.clone()))
            .collect()
    }

    fn uri_of(&self, path: &Path) -> Option<Url> {
        let key = canonical(path);
        self.docs
            .iter()
            .find(|(_, d)| canonical(&d.path) == key)
            .map(|(u, _)| u.clone())
            .or_else(|| Url::from_file_path(path).ok())
    }

    /// Check what changed and publish diagnostics.
    pub fn flush(&mut self) -> Outbox {
        if self.dirty.is_empty() && self.pending_clear.is_empty() {
            return Outbox::new();
        }
        self.dirty.clear();
        self.pending_clear.clear();
        let buffers = self.buffers();
        let mut coverage: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        let mut diagnostics: BTreeMap<Url, Vec<lsp_types::Diagnostic>> = BTreeMap::new();
        for (uri, doc) in &self.docs {
            let path = canonical(&doc.path);
            let covered = package_root(&path)
                .map(|root| canonical(&root))
                .and_then(|root| coverage.get(&root))
                .is_some_and(|loaded| loaded.is_empty() || loaded.contains(&path));
            if covered {
                continue;
            }
            let analysis = analyze(&doc.path, &doc.text, &buffers);
            if let Unit::Package(root) = &analysis.unit {
                coverage.insert(root.clone(), analysis.loaded.clone());
            }
            let by_path = convert(&analysis, &doc.path, &|path| self.uri_of(path));
            for (path, found) in by_path {
                if let Some(uri) = self.uri_of(&path) {
                    diagnostics.entry(uri).or_default().extend(found);
                }
            }
            diagnostics.entry(uri.clone()).or_default();
        }
        for uri in self.docs.keys().chain(self.published.keys()) {
            diagnostics.entry(uri.clone()).or_default();
        }
        self.publish_diagnostics(diagnostics)
    }

    fn publish_diagnostics(
        &mut self,
        diagnostics: BTreeMap<Url, Vec<lsp_types::Diagnostic>>,
    ) -> Outbox {
        let mut out = Outbox::new();
        let mut sent = BTreeMap::new();
        for (uri, mut diagnostics) in diagnostics {
            let mut seen = BTreeSet::new();
            diagnostics.retain(|diagnostic| seen.insert(json!(diagnostic).to_string()));
            let value = json!(&diagnostics);
            let version = self.docs.get(&uri).map(|doc| doc.version);
            let unchanged = self
                .published
                .get(&uri)
                .is_some_and(|previous| previous.0 == version && previous.1 == value);
            if !unchanged {
                out.push(publish(&uri, version, diagnostics));
            }
            if self.docs.contains_key(&uri) || value != json!([]) {
                sent.insert(uri, (version, value));
            }
        }
        self.published = sent;
        out
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

fn publish(uri: &Url, version: Option<i32>, diagnostics: Vec<lsp_types::Diagnostic>) -> Value {
    notification(
        "textDocument/publishDiagnostics",
        json!(PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version
        }),
    )
}

fn roots_of(p: &InitializeParams) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = p
        .workspace_folders
        .iter()
        .flatten()
        .filter_map(|f| f.uri.to_file_path().ok())
        .collect();
    #[allow(deprecated)]
    if let Some(root) = p.root_uri.as_ref().and_then(|u| u.to_file_path().ok()) {
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

fn initialize_result() -> Value {
    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                will_save: None,
                will_save_wait_until: None,
                save: Some(TextDocumentSyncSaveOptions::Supported(true)),
            },
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        definition_provider: Some(OneOf::Left(true)),
        workspace_symbol_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    };
    json!(InitializeResult {
        capabilities,
        server_info: Some(ServerInfo {
            name: "ginlsp".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned())
        })
    })
}

/// Serve LSP over `reader` and `writer` until `exit` or the end of input. Returns the process
/// exit code: 0 after `shutdown` then `exit`, else 1.
pub fn serve<R: Read, W: Write>(mut reader: BufReader<R>, mut writer: W) -> i32 {
    let mut server = Server::default();
    loop {
        if let Some(code) = server.exit_code() {
            return code;
        }
        // Check when the burst of input is over, not after every edit of it.
        if reader.buffer().is_empty() {
            for message in server.flush() {
                if write_message(&mut writer, &message).is_err() {
                    return 1;
                }
            }
        }
        let replies = match read_message(&mut reader) {
            Ok(message) => server.handle(&message),
            Err(ReadError::Eof) => {
                return if server.phase == Phase::ShuttingDown {
                    0
                } else {
                    1
                };
            }
            Err(ReadError::Io(_)) => return 1,
            Err(ReadError::Frame(m)) => vec![error_response(
                &Value::Null,
                PARSE_ERROR,
                &format!("bad message framing: {m}"),
            )],
            Err(ReadError::Json(m)) => vec![error_response(
                &Value::Null,
                PARSE_ERROR,
                &format!("invalid JSON: {m}"),
            )],
        };
        for message in replies {
            if write_message(&mut writer, &message).is_err() {
                return 1;
            }
        }
    }
}
