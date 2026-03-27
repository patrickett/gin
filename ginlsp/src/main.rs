mod capabilities;
mod diagnostics;
mod state;
mod util;

use capabilities::{
    build_binding_hover, build_completions, build_declare_hover, build_keyword_hover,
    build_local_binding_hover_with_narrowing_and_ast, build_self_hover,
    build_semantic_tokens_from_ast, build_signature_help, build_variant_hover, complete_flask_json,
    dot_completions, find_all_references, find_definition_range, is_flask_json_file,
    should_handle_file, use_completions, LEGEND_TYPE,
};
use dashmap::DashMap;
use diagnostics::symptoms_to_diagnostics;
use ginc::{ast::Tag, FileAst, typeck::TyEnv};
use state::{DocumentState, GinHost, JsonDocumentState};
use std::sync::{Arc, Mutex, RwLock};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use util::{get_char_at_position, get_number_at_position, get_word_at_position, is_in_comment};

const INFO: MessageType = MessageType::INFO;

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
            host: Arc::new(std::sync::Mutex::new(GinHost::new())),
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
        let ast = snapshot.parse(file);
        let symptoms = snapshot.diagnostics(file);
        if symptoms.is_empty() {
            self.ast_cache
                .insert(uri.to_string(), std::sync::Arc::new(ast));
        }
        let diagnostics = symptoms_to_diagnostics(source, &symptoms[..]);

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    /// Recursively evaluate an expression to a literal integer, resolving pattern variables
    /// via `infer_pattern_var_value`. Returns `None` if any sub-expression is non-literal.
    fn eval_expr_to_literal(expr: &ginc::ast::Expr, ast: &ginc::ast::FileAst) -> Option<i64> {
        Self::eval_expr_to_literal_with_locals(expr, ast, &[])
    }

    /// Like `eval_expr_to_literal` but also resolves simple variable references against
    /// a provided list of local bind expressions (e.g. a function body + if body).
    fn eval_expr_to_literal_with_locals(
        expr: &ginc::ast::Expr,
        ast: &ginc::ast::FileAst,
        locals: &[&ginc::ast::Expr],
    ) -> Option<i64> {
        use ginc::ast::{BindValue, Expr, Literal};
        use ginc::prelude::BinOp;
        match expr {
            Expr::Lit(Literal::Int(n)) => Some(*n),
            Expr::Lit(Literal::Number(n)) => Some(*n as i64),
            Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
                let var = call.path.root.as_str();
                // First: check provided local binds
                for local in locals {
                    if let Expr::Bind(b) = local {
                        if b.name().as_str() == var {
                            if let BindValue::Expr(e) = b.value() {
                                return Self::eval_expr_to_literal_with_locals(e, ast, locals);
                            }
                        }
                    }
                }
                // Fallback: pattern variable inference
                Self::infer_pattern_var_value(var, ast)?.parse::<i64>().ok()
            }
            Expr::Negate(inner) => {
                Some(-Self::eval_expr_to_literal_with_locals(inner, ast, locals)?)
            }
            Expr::Binary(bin) if !bin.op.is_comparison() => {
                let lhs = Self::eval_expr_to_literal_with_locals(&bin.lhs, ast, locals)?;
                let rhs = Self::eval_expr_to_literal_with_locals(&bin.rhs, ast, locals)?;
                match bin.op {
                    BinOp::Add => Some(lhs + rhs),
                    BinOp::Subtract => Some(lhs - rhs),
                    BinOp::Multiply => Some(lhs * rhs),
                    BinOp::Divide if rhs != 0 => Some(lhs / rhs),
                    BinOp::Modulo if rhs != 0 => Some(lhs % rhs),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Compute a rich return type string for a function bind using literal evaluation.
    /// Collects the outer return and any early returns from if blocks, resolving literal
    /// values where possible. Returns `None` if the bind is not a Body.
    fn compute_rich_return_type(
        bind: &ginc::ast::Bind,
        ast: &ginc::ast::FileAst,
    ) -> Option<String> {
        use ginc::ast::{BindValue, Expr};

        let BindValue::Body { exprs, ret } = bind.value() else {
            return None;
        };

        let mut types: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let push =
            |types: &mut Vec<String>, seen: &mut std::collections::HashSet<String>, val: String| {
                if seen.insert(val.clone()) {
                    types.push(val);
                }
            };

        // Outer return
        match &ret.0 {
            None => push(&mut types, &mut seen, "Nothing".to_string()),
            Some(expr) => {
                let outer_locals: Vec<&Expr> = exprs.iter().collect();
                if let Some(v) = Self::eval_expr_to_literal_with_locals(expr, ast, &outer_locals) {
                    push(&mut types, &mut seen, v.to_string());
                }
            }
        }

        // Early returns from if blocks
        for expr in exprs {
            if let Expr::If(if_expr) = expr {
                if let Some(ret_expr) = &if_expr.ret.0 {
                    let combined: Vec<&Expr> = exprs.iter().chain(if_expr.body.iter()).collect();
                    if let Some(v) =
                        Self::eval_expr_to_literal_with_locals(ret_expr, ast, &combined)
                    {
                        push(&mut types, &mut seen, v.to_string());
                    }
                }
            }
        }

        if types.is_empty() {
            return None;
        }
        Some(types.join(" or "))
    }

    fn find_local_bind_recursive<'a>(
        exprs: &'a [ginc::ast::Expr],
        name: &str,
    ) -> Option<&'a ginc::ast::Bind> {
        use ginc::ast::Expr;
        for expr in exprs {
            match expr {
                Expr::Bind(b) if b.name().as_str() == name => return Some(b),
                Expr::If(if_expr) => {
                    if let Some(found) = Self::find_local_bind_recursive(&if_expr.body, name) {
                        return Some(found);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// For a pattern variable like `v` in `if val is Some(v)`, returns its inferred literal
    /// value by tracing through the subject's type annotation.
    /// e.g. `val Maybe(3): Some(3)` → `v` maps to `3`.
    fn infer_pattern_var_value(var_name: &str, ast: &ginc::ast::FileAst) -> Option<String> {
        use ginc::ast::{BindValue, DeclareValue, Expr, IfCondition, Literal, Tag};

        for bind in ast.defs().values() {
            let BindValue::Body { exprs, .. } = bind.value() else {
                continue;
            };

            // Find an if-pattern that binds var_name, capture subject name + param position
            let pattern_info = exprs.iter().find_map(|e| {
                let Expr::If(if_expr) = e else { return None };
                let IfCondition::Pattern { subject, tag } = &if_expr.condition else {
                    return None;
                };
                let Tag::Generic(_, params, _) = tag else {
                    return None;
                };
                let param_pos = params.keys().position(|k| k.as_str() == var_name)?;
                // Subject is a simple variable reference
                let subject_name = match subject.as_ref() {
                    Expr::FnCall(c) if c.args.is_none() && c.path.segments.is_empty() => {
                        c.path.root.as_str()
                    }
                    _ => return None,
                };
                Some((subject_name.to_string(), tag.name().to_string(), param_pos))
            });

            let (subject_name, variant_name, param_pos) = pattern_info?;

            // Find the subject's local bind and its type_annotation
            let type_annotation = exprs.iter().find_map(|e| {
                let Expr::Bind(b) = e else { return None };
                if b.name().as_str() == subject_name {
                    b.type_annotation.as_ref()
                } else {
                    None
                }
            })?;

            let (type_name, type_args) = type_annotation;

            // Find the union declaration and the variant to get param-to-arg mapping
            let union_decl = ast
                .tags()
                .iter()
                .find(|(k, _)| k.as_str() == type_name.as_str())?
                .1;
            let DeclareValue::Union { variants } = union_decl.value() else {
                return None;
            };

            for variant in variants {
                let vtag = variant.tag();
                if vtag.name() != variant_name {
                    continue;
                }
                let Tag::Generic(_, variant_params, _) = vtag else {
                    continue;
                };
                // The variant param at param_pos links to a union type param by the same name
                let union_param_name = variant_params.keys().nth(param_pos)?;
                let type_param_pos = union_decl
                    .params()
                    .as_ref()?
                    .keys()
                    .position(|k| k == union_param_name)?;
                let arg = type_args.get(type_param_pos)?;
                return Some(match arg {
                    Expr::Lit(Literal::Int(n)) => n.to_string(),
                    Expr::Lit(Literal::Number(n)) => n.to_string(),
                    Expr::Lit(Literal::Float(f)) => f.to_string(),
                    _ => return None,
                });
            }
        }
        None
    }

    /// Build a variant display string with literal args substituted from the bind's TagCall.
    /// e.g. val: Maybe.Some(3) with variant=Some → "Some(3)"
    #[allow(dead_code)]
    fn variant_with_literal_args(
        bind: &ginc::ast::Bind,
        ast: &ginc::ast::FileAst,
        union_name: ginc::intern::IStr,
        variant_name: ginc::intern::IStr,
    ) -> Option<String> {
        use ginc::ast::{BindValue, DeclareValue, Expr, Literal, Tag};

        // Get the TagCall from the bind's value
        let tag_call = match bind.value() {
            BindValue::Expr(e) => {
                if let Expr::TagCall(tc) = e.as_ref() {
                    Some(tc)
                } else {
                    None
                }
            }
            _ => None,
        }?;

        // Find the variant definition in the union to get param order
        let decl = ast.tags().iter().find(|(k, _)| *k == &union_name)?.1;
        if let DeclareValue::Union { variants } = decl.value() {
            for variant in variants {
                let tag = variant.tag();
                if tag.name() == variant_name.as_str() {
                    return match tag {
                        Tag::Nominal(_, _) => Some(variant_name.as_str().to_string()),
                        Tag::Generic(_, vparams, _) => {
                            let args: Vec<String> = vparams
                                .keys()
                                .enumerate()
                                .map(|(i, _)| match tag_call.args.get(i) {
                                    Some(Expr::Lit(Literal::Int(n))) => n.to_string(),
                                    Some(Expr::Lit(Literal::Number(n))) => n.to_string(),
                                    Some(Expr::Lit(Literal::Float(f))) => f.to_string(),
                                    _ => "_".to_string(),
                                })
                                .collect();
                            Some(format!("{}({})", variant_name.as_str(), args.join(", ")))
                        }
                        Tag::Qualified(_) => Some(variant_name.as_str().to_string()),
                    };
                }
            }
        }
        None
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
            document_formatting_provider: Some(OneOf::Left(true)),
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
            self.ast_cache.remove(&uri);
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

            // Check for dot completions (after typing `.`)
            // The cursor position is after the dot, so we check the character before it
            let before_dot_position = Position {
                line: position.line,
                character: position.character.saturating_sub(1),
            };
            if let Some(ch) = get_char_at_position(&state.source, before_dot_position) {
                if ch == '.' {
                    let snapshot = self.snapshot();
                    let ast = snapshot.parse(state.file);
                    // Use cached AST if the current parse is empty (e.g., due to incomplete syntax)
                    let ast = if ast.tags().is_empty() && ast.defs().is_empty() {
                        self.ast_cache
                            .get(&uri.to_string())
                            .map(|r| std::sync::Arc::clone(&r))
                            .unwrap_or_else(|| std::sync::Arc::new(ast))
                    } else {
                        std::sync::Arc::new(ast)
                    };
                    if let Some(items) = dot_completions(&state.source, position, &ast) {
                        return Ok(Some(CompletionResponse::Array(items)));
                    }
                }
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
            if is_in_comment(&state.source, position) {
                return Ok(None);
            }

            let snapshot = self.snapshot();
            let ast = snapshot.parse(state.file);
            // Arc avoids a deep clone on fallback to the last error-free AST.
            let ast = if ast.tags().is_empty() && ast.defs().is_empty() {
                self.ast_cache
                    .get(&uri.to_string())
                    .map(|r| std::sync::Arc::clone(&r))
                    .unwrap_or_else(|| std::sync::Arc::new(ast))
            } else {
                std::sync::Arc::new(ast)
            };
            let file_path = state.file.path(&snapshot.db);
            let module = self.compute_module_path(&file_path, &uri);

            if let Some(ch) = get_char_at_position(&state.source, position) {
                match ch {
                    '(' | ')' | '[' | ']' => return Ok(None),
                    ':' => {
                        if let Some(value) = build_keyword_hover(":") {
                            return Ok(Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value,
                                }),
                                range: None,
                            }));
                        }
                    }
                    _ => {}
                }
            }

            if let Some(num) = get_number_at_position(&state.source, position) {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```gin\n{num}\n```"),
                    }),
                    range: None,
                }));
            }

            if let Some(word) = get_word_at_position(&state.source, position) {
                let value = hover_for_word(&word, &state.source, position, &ast, &module);
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    }),
                    range: None,
                }));
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

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri.to_string();

        if let Some(state) = self.documents.get(&uri) {
            let formatted = ginfmt::format(&state.source);

            if formatted == *state.source {
                return Ok(None); // No changes needed
            }

            // Replace entire document
            let full_range = Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: state.source.lines().count() as u32,
                    character: state
                        .source
                        .lines()
                        .last()
                        .map(|l| l.len() as u32)
                        .unwrap_or(0),
                },
            };

            Ok(Some(vec![TextEdit {
                range: full_range,
                new_text: formatted,
            }]))
        } else {
            Ok(None)
        }
    }
}

/// Convert an LSP Position to a byte offset in the source string.
fn source_byte_offset(source: &str, position: tower_lsp::lsp_types::Position) -> usize {
    let mut offset = 0;
    for (line_idx, line) in source.lines().enumerate() {
        if line_idx == position.line as usize {
            offset += position.character as usize;
            break;
        }
        offset += line.len() + 1; // +1 for the newline
    }
    offset.min(source.len())
}

/// Scan backward from `cursor_byte` to find the receiver type of the enclosing method.
///
/// Looks for a line of the form `TypeName.method_name:` or `TypeName.method_name(...):`.
/// Returns the type name as a `&str` slice into `source`.
fn enclosing_method_receiver(source: &str, cursor_byte: usize) -> Option<&str> {
    let before = &source[..cursor_byte];
    for line in before.lines().rev() {
        let trimmed = line.trim_start();
        if let Some(dot_pos) = trimmed.find('.') {
            let type_part = &trimmed[..dot_pos];
            if type_part
                .chars()
                .next()
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
                && type_part.chars().all(|c| c.is_alphanumeric())
            {
                return Some(type_part);
            }
        }
    }
    None
}

/// Returns true if `cursor_byte` is inside parentheses that follow an uppercase tag name.
///
/// Used to detect pattern variables like `x` in `is Some(x) then ...`.
fn is_in_tag_params(source: &str, cursor_byte: usize) -> bool {
    let before = &source[..cursor_byte.min(source.len())];
    let bytes = before.as_bytes();
    let mut depth = 0i32;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    let before_paren = before[..i].trim_end();
                    let id_end = before_paren.len();
                    let id_start = before_paren
                        .rfind(|c: char| !c.is_alphanumeric() && c != '_')
                        .map(|p| p + 1)
                        .unwrap_or(0);
                    let ident = &before_paren[id_start..id_end];
                    return ident
                        .chars()
                        .next()
                        .map(|c| c.is_uppercase())
                        .unwrap_or(false);
                } else {
                    depth -= 1;
                }
            }
            _ => {}
        }
    }
    false
}

#[allow(dead_code)]
/// Find the narrowed type for a variable based on control-flow analysis.
pub(crate) fn find_narrowed_type_at_position(
    var_name: &str,
    position: Position,
    source: &str,
    flow_result: &ginc::typeck::FlowAnalysis,
    ast: &ginc::ast::FileAst,
    local_bind: Option<&ginc::ast::Bind>,
) -> Option<String> {
    let if_pattern = format!("if {} is ", var_name);

    let if_line = source
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains(&if_pattern))
        .map(|(i, _)| i)?;

    let if_indent = source
        .lines()
        .nth(if_line)
        .map(|l| l.len() - l.trim_start().len())
        .unwrap_or(0);

    let if_return_line = source
        .lines()
        .enumerate()
        .skip(if_line + 1)
        .find(|(_, line)| {
            line.trim().starts_with("return")
                && line.len() - line.trim_start().len() <= if_indent
        })
        .map(|(i, _)| i)?;

    if position.line <= if_line as u32 {
        return None;
    }
    if position.line <= if_return_line as u32 {
        let (union_name, variant_name) = flow_result.inside_if_variant(var_name)?;
        let display = local_bind
            .and_then(|b| Backend::variant_with_literal_args(b, ast, union_name, variant_name))
            .unwrap_or_else(|| variant_name.as_str().to_string());
        return Some(display);
    }

    let raw_narrowed = flow_result.narrowed_type_string(var_name)?;
    let variant_only = raw_narrowed
        .find('.')
        .map(|dot| raw_narrowed[dot + 1..].to_string())
        .unwrap_or(raw_narrowed);
    Some(variant_only)
}

/// Run the full hover cascade for a word at a given position.
/// Returns the markdown hover string (including the `error: unknown` fallback).
pub(crate) fn hover_for_word(
    word: &str,
    source: &str,
    position: Position,
    ast: &ginc::ast::FileAst,
    module: &str,
) -> String {
    use ginc::ast::{BindValue, DeclareValue, ParameterKind, Variant};

    let ty_env = TyEnv::from_file_ast(ast);

    // All-digit words are numeric literals (the numeric path in `hover` handles -/. prefixes,
    // but plain digits also arrive here when called directly, e.g. from tests).
    if word.chars().all(|c| c.is_ascii_digit()) {
        return format!("```gin\n{word}\n```");
    }

    if word == "self" {
        let byte_pos = source_byte_offset(source, position);
        if let Some(recv_name) = enclosing_method_receiver(source, byte_pos) {
            let decl = ast.tags().iter().find(|(k, _)| k.as_str() == recv_name).map(|(_, v)| v);
            return build_self_hover(recv_name, decl);
        }
    }

    for decl in ast.tags().values() {
        if let DeclareValue::Union { variants } = decl.value() {
            for variant in variants {
                let (tag, doc) = match variant {
                    Variant::External(tag) => (tag, None),
                    Variant::Local { tag, doc_comment } => (tag, doc_comment.as_ref()),
                };
                if tag.name() == word {
                    return build_variant_hover(module, &tag.to_string(), doc);
                }
            }
        }
    }

    for (name, decl) in ast.tags() {
        if name.as_str() == word {
            let ty = ty_env.resolve_tag(&Tag::Nominal(*name, ginc::prelude::SimpleSpan::from(0..0)));
            return build_declare_hover(module, decl, decl.doc_comment(), Some(&ty));
        }
    }

    for (name, bind) in ast.defs() {
        let name_str = name.as_str();
        let matches = name_str == word
            || (name_str.contains('.') && name_str.split('.').next_back() == Some(word));
        if matches {
            let rich_ret = Backend::compute_rich_return_type(bind, ast);
            return build_binding_hover(module, bind, ast.defs(), rich_ret);
        }
    }

    for bind in ast.defs().values() {
        if let BindValue::Body { exprs, .. } = bind.value() {
            if let Some(local_bind) = Backend::find_local_bind_recursive(exprs, word) {
                let literal_type = Backend::eval_expr_to_literal(
                    match local_bind.value() {
                        BindValue::Expr(e) => e.as_ref(),
                        _ => return format!("```gin\n{word}\n```"),
                    },
                    ast
                ).map(|v| v.to_string());
                return build_local_binding_hover_with_narrowing_and_ast(local_bind, literal_type.as_deref(), Some(ast));
            }
        }
    }

    // Parameter names and their types in any function definition
    for bind in ast.defs().values() {
        if let Some(params) = bind.params() {
            for (param_name, kind) in params {
                if param_name.as_str() == word {
                    let ty_str = match kind {
                        ParameterKind::Generic => word.to_string(),
                        ParameterKind::Tagged(tag) => format!("{word} {tag}"),
                        ParameterKind::Default(_) => word.to_string(),
                    };
                    return format!("```gin\n{ty_str}\n```");
                }
                // Also match type names used as parameter types
                if let ParameterKind::Tagged(tag) = kind {
                    if tag.name() == word {
                        // It's a type reference — look it up as a tag
                        if let Some(decl) = ast.tags().get(&ginc::intern::IStr::new(word.to_string())) {
                            let ty = ty_env.resolve_tag(&Tag::Nominal(ginc::intern::IStr::new(word.to_string()), ginc::prelude::SimpleSpan::from(0..0)));
                            return build_declare_hover(module, decl, decl.doc_comment(), Some(&ty));
                        }
                        return format!("```gin\n{word}\n```");
                    }
                }
            }
        }
    }

    if let Some(value) = build_keyword_hover(word) {
        return value;
    }

    if let Some(inferred) = Backend::infer_pattern_var_value(word, ast) {
        return format!("```gin\n{word} {inferred}\n```");
    }

    let byte_pos = source_byte_offset(source, position);
    if word.chars().all(|c| c.is_alphabetic() && c.is_lowercase() || c == '_')
        && is_in_tag_params(source, byte_pos)
    {
        return format!("```gin\n{word}\n```");
    }

    format!("error: unknown `{word}`")
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
    use super::*;
    use ginc::parse::parse_from_str;

    fn all_words_with_positions(source: &str) -> Vec<(String, Position)> {
        let mut out = Vec::new();
        for (line_idx, line) in source.lines().enumerate() {
            let bytes = line.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if util::is_identifier_char(bytes[i] as char) {
                    let start = i;
                    while i < bytes.len() && util::is_identifier_char(bytes[i] as char) {
                        i += 1;
                    }
                    out.push((
                        line[start..i].to_string(),
                        Position { line: line_idx as u32, character: start as u32 },
                    ));
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    fn assert_no_unknown_hover(source: &str) {
        let ast = parse_from_str(source);

        let mut unknowns = Vec::new();
        for (word, position) in all_words_with_positions(source) {
            let hover = hover_for_word(&word, source, position, &ast, "test");
            if hover.contains("error: unknown") {
                unknowns.push(word);
            }
        }

        assert!(
            unknowns.is_empty(),
            "words with unknown hover: {:?}\nsource:\n{source}",
            unknowns
        );
    }

    #[test]
    fn hover_simple_function() {
        assert_no_unknown_hover("f(x Int) Int:\n    x\nreturn\n");
    }

    #[test]
    fn hover_union_declaration() {
        assert_no_unknown_hover("Maybe(x) is Some(x) or None\n");
    }

    #[test]
    fn hover_function_with_if_pattern() {
        assert_no_unknown_hover(
            "Maybe(x) is Some(x) or None\n\nmain:\n    val Maybe(3): Some(3)\n    if val is Some(v)\n        add_one: v + 1\n    return add_one\n    val\nreturn\n",
        );
    }

    #[test]
    fn dot_completion_union_variants() {
        use capabilities::dot_completions;
        use tower_lsp::lsp_types::Position;

        // Create AST with Maybe union definition
        let source = "Maybe(x) is Some(x) or None";
        let ast = parse_from_str(source);

        // Test completion after `Maybe.` (direct type completion)
        let source_with_dot = "Maybe.";
        let position = Position { line: 0, character: 6 }; // `Maybe.|`
        let completions = dot_completions(source_with_dot, position, &ast);

        assert!(completions.is_some(), "Expected some completions for `Maybe.`");

        let items = completions.unwrap();
        assert_eq!(items.len(), 2, "Expected 2 variants for Maybe type");

        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"Some(x)"), "Expected 'Some(x)' variant");
        assert!(labels.contains(&"None"), "Expected 'None' variant");

        // Check detail text for Some variant (should show the full qualified name with parameter)
        let some_item = items.iter().find(|i| i.label == "Some(x)").unwrap();
        assert_eq!(some_item.detail.as_ref().unwrap(), &"Maybe.Some(x)");
    }
}
