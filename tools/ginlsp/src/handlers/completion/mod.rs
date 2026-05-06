pub(crate) mod json;
mod path;

use crate::Backend;
use ast::FileAst;
use database::{file_parse_output, intern_package_files, package_ty_env, sorted_package_files};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::Ty;
use typeck::{completions_for_ast, dot_type_at, position_to_byte_offset, CompletionKind};

impl Backend {
    pub(crate) async fn handle_completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let doc_uri = params.text_document_position.text_document.uri.clone();
        let uri = doc_uri.to_string();
        let position = params.text_document_position.position;

        #[cfg(debug_assertions)]
        self.client
            .log_message(
                MessageType::INFO,
                format!("completion requested for URI: {}", uri),
            )
            .await;

        if let Some(state) = self.json_documents.get(&uri) {
            let items = json::complete_flask_json(&state.source, position, &doc_uri);
            #[cfg(debug_assertions)]
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Returning {} completions for flask.jsonc", items.len()),
                )
                .await;
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // Snapshot what we need from the document store and drop the DashMap
        // ref before any await: `Ref` holds a shard read-lock and is `!Send`.
        let (source, file) = match self.documents.get(&uri) {
            Some(state) => (state.source.clone(), state.file),
            None => {
                #[cfg(debug_assertions)]
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("No document found for URI: {}", uri),
                    )
                    .await;
                return Ok(None);
            }
        };

        // `use` import path completion is filesystem-only and cheap; keep it
        // on the async runtime so the common case stays fast.
        let config = self.get_or_load_config(&doc_uri);
        if let Some(items) = path::use_completions(&source, position, &doc_uri, config.as_ref()) {
            return Ok(Some(CompletionResponse::Array(items)));
        }

        // Heavy section: parse + package-wide TyEnv. Off-load so a wedged
        // Salsa query (e.g. parser hang on `core.`) cannot pin an async worker.
        let result = self
            .run_blocking_request("completion", move |this| {
                compute_completions(&this, doc_uri, source, file, position)
            })
            .await;

        Ok(result.map(CompletionResponse::Array))
    }
}

/// Synchronous completion compute, intended to run on the blocking pool.
fn compute_completions(
    backend: &Backend,
    doc_uri: Url,
    source: String,
    file: database::File,
    position: Position,
) -> Vec<CompletionItem> {
    let snapshot = backend.snapshot();
    let ast = file_parse_output(&snapshot.db, file).ast.clone();

    if let Some(byte_pos) = position_to_byte_offset(&source, position.line, position.character) {
        let pkg_root = backend.package_root_for_uri(&doc_uri);
        let all_files = if let Some(root) = &pkg_root {
            let mut host = backend.lock_host();
            host.load_package(root).files
        } else {
            vec![file]
        };
        let package_files = sorted_package_files(&snapshot.db, &all_files);
        let pkg = intern_package_files(&snapshot.db, package_files);
        let ty_env = package_ty_env(&snapshot.db, pkg);

        if let Some(ty) = dot_type_at(&source, &ast, &ty_env, byte_pos) {
            let items = dot_completions(ty);
            if !items.is_empty() {
                return items;
            }
        }
    }

    build_completions(&ast)
}

pub(crate) fn build_completions(ast: &FileAst) -> Vec<CompletionItem> {
    completions_for_ast(ast)
        .into_iter()
        .map(|c| {
            let kind = match c.kind {
                CompletionKind::Function => CompletionItemKind::FUNCTION,
                CompletionKind::Variable => CompletionItemKind::VARIABLE,
                CompletionKind::Tag => CompletionItemKind::CLASS,
                CompletionKind::Keyword => CompletionItemKind::KEYWORD,
            };
            let documentation = c.documentation.map(|doc| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: doc,
                })
            });
            CompletionItem {
                label: c.label,
                kind: Some(kind),
                detail: c.detail,
                documentation,
                ..Default::default()
            }
        })
        .collect()
}

pub(crate) fn dot_completions(ty: Ty) -> Vec<CompletionItem> {
    let Ty::Union { name, variants } = ty else {
        return vec![];
    };
    let qualifier = name.as_str().to_string();
    variants
        .iter()
        .map(|(variant_name, fields)| {
            let label = if fields.is_empty() {
                variant_name.to_string()
            } else {
                let names: Vec<String> = fields.iter().map(|(n, _)| n.to_string()).collect();
                format!("{}({})", variant_name, names.join(", "))
            };
            let detail = format!("{}.{}", qualifier, label);
            CompletionItem {
                label: label.clone(),
                insert_text: Some(label),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(detail),
                ..Default::default()
            }
        })
        .collect()
}
