use dashmap::DashMap;
use ginc::frontend::lexer::HasSemanticTokenType;
use ropey::Rope;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

// if builtin text in the file, then provide autocomplete for builtins
// otherwise probably not important

#[derive(Debug, Clone)]
struct ImCompleteSemanticToken {
    start: usize,
    length: usize,
    token_type: usize, // index into LEGEND_TYPE
}

#[derive(Debug)]
struct Backend {
    client: Client,
    document_map: DashMap<String, String>,
    token_map: DashMap<String, Vec<ImCompleteSemanticToken>>,
}

const INFO: MessageType = MessageType::INFO;

pub const LEGEND_TYPE: &[SemanticTokenType] = &[
    SemanticTokenType::FUNCTION,
    SemanticTokenType::STRUCT,
    SemanticTokenType::COMMENT,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::CLASS,
    SemanticTokenType::NUMBER,
];

/// Very small lexer that finds a few keywords and identifiers.
fn parse_tokens(text: &str) -> Vec<ImCompleteSemanticToken> {
    // Simple regex to find a handful of tokens.
    // let keyword_regex = Regex::new(r"\b(fn|let|mut|if|else|for|while|return)\b").unwrap();
    let mut tokens = Vec::new();

    // for mat in keyword_regex.find_iter(text) {
    //     let token_type_index = if mat.as_str() == "fn" {
    //         0 // FUNCTION
    //     } else if ["let", "mut"].contains(&mat.as_str()) {
    //         1 // VARIABLE
    //     } else if ["if", "else", "for", "while", "return"].contains(&mat.as_str()) {
    //         5 // KEYWORD
    //     } else {
    //         0
    //     };

    //     tokens.push(ImCompleteSemanticToken {
    //         start: mat.start(),
    //         length: mat.end() - mat.start(),
    //         token_type: token_type_index,
    //     });
    // }
    let lex = ginc::frontend::lexer::GinLexer::new(text);

    for (tok, span) in lex {
        if let Some(token_type) = tok.semantic_token_type_index() {
            tokens.push(ImCompleteSemanticToken {
                start: span.start,
                length: span.end - span.start,
                token_type,
            })
        }
    }

    tokens
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        let gin_file_doc_filter = DocumentFilter {
            language: Some("gin".to_string()),
            scheme: Some("file".to_string()),
            pattern: None,
        };

        let manifest_doc_filter = DocumentFilter {
            language: Some("json".to_string()),
            scheme: None,
            pattern: Some("**/flask.json".to_string()),
        };

        let capabilities = ServerCapabilities {
            semantic_tokens_provider: Some(
                SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(
                    SemanticTokensRegistrationOptions {
                        text_document_registration_options: {
                            TextDocumentRegistrationOptions {
                                document_selector: Some(vec![
                                    gin_file_doc_filter,
                                    manifest_doc_filter,
                                ]),
                            }
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
            ..Default::default()
        };

        let init_result = InitializeResult {
            capabilities,
            ..Default::default()
        };
        Ok(init_result)
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(INFO, "gin language server initialized!")
            .await;
    }

    // ----------------------------------------------------------------------
    // Document change handling – store text and re‑parse tokens
    // ----------------------------------------------------------------------
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        let text = params.text_document.text.clone();

        self.document_map.insert(uri.clone(), text.clone());

        let tokens = parse_tokens(&text);
        self.token_map.insert(uri.clone(), tokens);

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("did open: {:#?}", params))
                .await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        if let Some(change) = params.content_changes.first() {
            let text = change.text.clone();

            self.document_map.insert(uri.clone(), text.clone());

            let tokens = parse_tokens(&text);
            self.token_map.insert(uri.clone(), tokens);
        }

        #[cfg(debug_assertions)]
        {
            self.client
                .log_message(INFO, format!("did change: {:#?}", params))
                .await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri.to_string();
        if let Some(text) = &params.text {
            let text = text.clone();

            self.document_map.insert(uri.clone(), text.clone());

            let tokens = parse_tokens(&text);
            self.token_map.insert(uri.clone(), tokens);

            // Tell the client that semantic tokens may have changed.
            let _ = self.client.semantic_tokens_refresh().await;
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

        // Optional: drop the stored data when a document is closed.
        self.document_map.remove(&uri);
        self.token_map.remove(&uri);

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

    // ----------------------------------------------------------------------
    // Semantic‑token support
    // ----------------------------------------------------------------------
    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.to_string();

        // Retrieve stored text and tokens for the document.
        let (text_opt, tokens_opt) = {
            let text_opt = self.document_map.get(&uri).map(|v| v.clone());
            let tokens_opt = self.token_map.get(&uri).map(|v| v.clone());
            (text_opt, tokens_opt)
        };

        if let Some(text) = text_opt {
            if let Some(mut tokens) = tokens_opt {
                // LSP expects tokens sorted by absolute start position.
                tokens.sort_by(|a, b| a.start.cmp(&b.start));

                let rope = Rope::from_str(&text);
                let mut pre_line: u32 = 0;
                let mut pre_start: u32 = 0;

                // Convert to LSP’s delta‑encoded format.
                let semantic_tokens: Vec<SemanticToken> = tokens
                    .iter()
                    .filter_map(|token| {
                        let line = rope.try_byte_to_line(token.start).ok()? as u32;
                        let first_char_of_line = rope.try_line_to_char(line as usize).ok()? as u32;
                        let start_in_line =
                            rope.try_byte_to_char(token.start).ok()? as u32 - first_char_of_line;
                        let delta_line = line - pre_line;
                        let delta_start = if delta_line == 0 {
                            start_in_line - pre_start
                        } else {
                            start_in_line
                        };

                        let ret = SemanticToken {
                            delta_line,
                            delta_start,
                            length: token.length as u32,
                            token_type: token.token_type as u32,
                            token_modifiers_bitset: 0,
                        };
                        pre_line = line;
                        pre_start = start_in_line;

                        Some(ret)
                    })
                    .collect();

                return Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
                    result_id: None,
                    data: semantic_tokens,
                })));
            }
        }

        Ok(None)
    }

    // async fn semantic_tokens_range(
    //     &self,
    //     params: SemanticTokensRangeParams,
    // ) -> Result<Option<SemanticTokensRangeResult>> {
    //     let uri = params.text_document.uri.to_string();
    //     let semantic_tokens = || -> Option<Vec<SemanticToken>> {
    //         let im_complete_tokens = self.token_map.get(&uri)?;
    //         let rope = self.document_map.get(&uri)?;
    //         let mut pre_line = 0;
    //         let mut pre_start = 0;
    //         let semantic_tokens = im_complete_tokens
    //             .iter()
    //             .filter_map(|token| {
    //                 let line = rope.try_byte_to_line(token.start).ok()? as u32;
    //                 let first = rope.try_line_to_char(line as usize).ok()? as u32;
    //                 let start = rope.try_byte_to_char(token.start).ok()? as u32 - first;
    //                 let ret = Some(SemanticToken {
    //                     delta_line: line - pre_line,
    //                     delta_start: if start >= pre_start {
    //                         start - pre_start
    //                     } else {
    //                         start
    //                     },
    //                     length: token.length as u32,
    //                     token_type: token.token_type as u32,
    //                     token_modifiers_bitset: 0,
    //                 });
    //                 pre_line = line;
    //                 pre_start = start;
    //                 ret
    //             })
    //             .collect::<Vec<_>>();
    //         Some(semantic_tokens)
    //     }();
    //     Ok(semantic_tokens.map(|data| {
    //         SemanticTokensRangeResult::Tokens(SemanticTokens {
    //             result_id: None,
    //             data,
    //         })
    //     }))
    // }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    // Initialise the in‑memory stores when creating the LSP service.
    let (service, socket) = LspService::new(|client| Backend {
        client,
        document_map: DashMap::new(),
        token_map: DashMap::new(),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
