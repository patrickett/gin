use crate::Backend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

impl Backend {
    pub(crate) async fn handle_initialize(
        &self,
        _: InitializeParams,
    ) -> Result<InitializeResult> {
        let capabilities = ServerCapabilities {
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
                    "/".to_string(),
                    ":".to_string(),
                    "\"".to_string(),
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

    pub(crate) async fn handle_initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "gin language server initialized!")
            .await;
    }

    pub(crate) async fn handle_shutdown(&self) -> Result<()> {
        self.client
            .log_message(MessageType::INFO, "gin language server shutting down!")
            .await;
        Ok(())
    }
}
