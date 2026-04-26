pub(crate) mod json;
mod path;

use crate::Backend;
use ast::FileAst;
use analyze::{intern_package_files, package_ty_env, sorted_package_files};
use database::file_parse_output;
use ide::{completions_for_ast, dot_type_at, position_to_byte_offset, CompletionKind};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::Ty;

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
            let items = json::complete_flask_json(
                &state.source,
                position,
                &params.text_document_position.text_document.uri,
            );
            #[cfg(debug_assertions)]
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("Returning {} completions for flask.jsonc", items.len()),
                )
                .await;
            return Ok(Some(CompletionResponse::Array(items)));
        }

        if let Some(state) = self.documents.get(&uri) {
            let config = self.get_or_load_config(&params.text_document_position.text_document.uri);

            if let Some(items) = path::use_completions(
                &state.source,
                position,
                &params.text_document_position.text_document.uri,
                config.as_ref(),
            ) {
                return Ok(Some(CompletionResponse::Array(items)));
            }

            if let Some(byte_pos) =
                position_to_byte_offset(&state.source, position.line, position.character)
            {
                let snapshot = self.snapshot();
                let ast = file_parse_output(&snapshot.db, state.file).ast.clone();
                let pkg_root = self.package_root_for_uri(&doc_uri);
                let all_files = if let Some(root) = &pkg_root {
                    let mut host = self.lock_host();
                    host.load_package(root).files
                } else {
                    vec![state.file]
                };
                let package_files = sorted_package_files(&snapshot.db, &all_files);
                let pkg = intern_package_files(&snapshot.db, package_files);
                let ty_env = package_ty_env(&snapshot.db, pkg);

                if let Some(ty) = dot_type_at(&state.source, &ast, &ty_env, byte_pos) {
                    let items = dot_completions(ty);
                    if !items.is_empty() {
                        return Ok(Some(CompletionResponse::Array(items)));
                    }
                }
            }

            let snapshot = self.snapshot();
            let ast = file_parse_output(&snapshot.db, state.file).ast.clone();
            return Ok(Some(CompletionResponse::Array(build_completions(&ast))));
        }

        #[cfg(debug_assertions)]
        self.client
            .log_message(
                MessageType::INFO,
                format!("No document found for URI: {}", uri),
            )
            .await;

        Ok(None)
    }
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
