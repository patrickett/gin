#[cfg(test)]
use diagnostic::code::FormatCode;
use std::path::Path;
use std::sync::Arc;

use ginfmt::help::{Edit, Help, HelpContext, non_overlapping};
use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::handlers::{byte_range_to_range, single_file_action};

impl Backend {
    pub(crate) fn style_code_actions(&self, params: &CodeActionParams) -> Vec<CodeActionOrCommand> {
        let uri = &params.text_document.uri;
        let Some(path) = Self::file_path_from_uri(uri) else {
            return Vec::new();
        };
        let Some((source, helps)) = self.style_help(&path, |context| context.suggestions()) else {
            return Vec::new();
        };
        Self::style_actions_from_help(
            uri,
            &source,
            params.range,
            params.context.only.as_deref(),
            helps,
        )
    }

    pub(crate) fn redundant_binding_symptoms(&self, path: &Path) -> Vec<diagnostic::Diagnostic> {
        self.style_help(path, |context| {
            context.redundant_destructure_binding_suggestions()
        })
        .map(|(_, helps)| helps.into_iter().map(|help| help.diagnostic).collect())
        .unwrap_or_default()
    }

    fn style_help(
        &self,
        path: &Path,
        suggestions: impl FnOnce(&HelpContext<'_>) -> Vec<Help>,
    ) -> Option<(Arc<String>, Vec<Help>)> {
        let snapshot = self.snapshot();
        let (source, parsed) = snapshot.cache.source_and_parse(path)?;
        let package = snapshot
            .cache
            .get_or_compute_package(&[path.to_path_buf()])?;
        let index = package.file_paths.iter().position(|file| file == path)?;
        let definitions: Vec<_> = package.typed_asts.iter().collect();
        let context = HelpContext {
            source: &source,
            parsed: &parsed,
            typed: &package.typed_asts[index],
            package: &definitions,
        };
        let helps = suggestions(&context);
        Some((source, helps))
    }

    pub(crate) fn style_actions_from_help(
        uri: &Url,
        source: &str,
        selection: Range,
        only: Option<&[CodeActionKind]>,
        helps: Vec<Help>,
    ) -> Vec<CodeActionOrCommand> {
        let permits = |kind: &CodeActionKind| {
            only.is_none_or(|only| {
                only.is_empty()
                    || only.iter().any(|requested| {
                        kind.as_str() == requested.as_str()
                            || kind
                                .as_str()
                                .starts_with(&format!("{}.", requested.as_str()))
                    })
            })
        };
        let edit = |edit: &Edit| {
            TextEdit::new(
                byte_range_to_range(edit.span.start(), edit.span.end(), source),
                edit.replacement.clone(),
            )
        };
        let action = |title: String, kind: CodeActionKind, edits: Vec<TextEdit>| {
            CodeActionOrCommand::CodeAction(single_file_action(uri, title, kind, edits))
        };
        let mut actions = Vec::new();
        if permits(&CodeActionKind::QUICKFIX) {
            for help in &helps {
                if help.edits.is_empty() {
                    continue;
                }
                let diagnostic_range = byte_range_to_range(
                    help.diagnostic.span.start(),
                    help.diagnostic.span.end(),
                    source,
                );
                if diagnostic_range.start <= selection.end
                    && selection.start <= diagnostic_range.end
                {
                    actions.push(action(
                        help.diagnostic
                            .help
                            .clone()
                            .unwrap_or_else(|| help.diagnostic.message.clone()),
                        CodeActionKind::QUICKFIX,
                        help.edits.iter().map(&edit).collect(),
                    ));
                }
            }
        }
        let fix_all = CodeActionKind::new("source.fixAll.ginfmt");
        if helps.iter().any(|help| !help.edits.is_empty()) && permits(&fix_all) {
            let edits = non_overlapping(helps)
                .iter()
                .flat_map(|help| &help.edits)
                .map(edit)
                .collect();
            actions.push(action(
                "Apply all Gin style fixes".to_owned(),
                fix_all,
                edits,
            ));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower_lsp::LspService;

    fn params(
        uri: Url,
        range: Range,
        diagnostics: Vec<Diagnostic>,
        only: Option<Vec<CodeActionKind>>,
    ) -> CodeActionParams {
        CodeActionParams {
            text_document: TextDocumentIdentifier::new(uri),
            range,
            context: CodeActionContext {
                diagnostics,
                only,
                trigger_kind: None,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        }
    }

    #[tokio::test]
    async fn generic_alias_has_a_published_hint_and_edits_for_quickfix_and_fix_all() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path = std::env::temp_dir().join(format!("gin-alias-style-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        let source = concat!(
            "Holder(t) has\n",
            "    storage t\n",
            "    release(self):\n",
            "        Holder(storage) := self\n",
            "        region: storage\n",
            "        region.release\n",
            "        return\n",
        );
        backend
            .snapshot()
            .cache
            .set_contents(&path, source.to_owned());
        let diagnostics = backend.compute_package_diagnostics(None, None, path.clone());
        let (_, diagnostics) = diagnostics
            .iter()
            .find(|(document, _)| document == &uri)
            .unwrap();
        let hint = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code
                    == Some(NumberOrString::String(
                        "fmt-redundant-destructure-binding".to_owned(),
                    ))
            })
            .unwrap_or_else(|| panic!("{diagnostics:?}"));
        assert_eq!(hint.severity, Some(DiagnosticSeverity::HINT));
        assert_eq!(
            hint.range,
            Range::new(Position::new(4, 8), Position::new(4, 14))
        );

        let params = assert_alias_code_actions(backend, &uri, hint);
        let fixed = source
            .replace("Holder(storage)", "Holder(storage: region)")
            .replace("        region: storage\n", "");
        backend.snapshot().cache.set_contents(&path, fixed);
        assert!(backend.redundant_binding_symptoms(&path).is_empty());
        assert!(backend.style_code_actions(&params).is_empty());
    }

    fn assert_alias_code_actions(
        backend: &Backend,
        uri: &Url,
        hint: &Diagnostic,
    ) -> CodeActionParams {
        let mut params = params(
            uri.clone(),
            hint.range,
            vec![hint.clone()],
            Some(vec![CodeActionKind::QUICKFIX]),
        );
        for kind in [CodeActionKind::QUICKFIX, CodeActionKind::SOURCE_FIX_ALL] {
            params.context.only = Some(vec![kind]);
            let actions = backend.style_code_actions(&params);
            assert_eq!(actions.len(), 1, "{actions:?}");
            let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
                panic!("code action")
            };
            assert_eq!(
                action.edit.as_ref().unwrap().changes.as_ref().unwrap()[uri],
                [
                    TextEdit::new(
                        Range::new(Position::new(3, 15), Position::new(3, 22)),
                        "storage: region".to_owned()
                    ),
                    TextEdit::new(
                        Range::new(Position::new(4, 0), Position::new(5, 0)),
                        String::new()
                    ),
                ]
            );
        }
        params
    }

    #[test]
    fn advisory_help_does_not_offer_empty_code_actions() {
        let helps = vec![Help {
            diagnostic: diagnostic::Diagnostic::new(
                FormatCode::RedundantDestructureBinding,
                "alias",
            )
            .with_category(diagnostic::Category::Help),
            edits: Vec::new(),
        }];
        assert!(
            Backend::style_actions_from_help(
                &Url::from_file_path("/alias.gin").unwrap(),
                "",
                Range::default(),
                None,
                helps,
            )
            .is_empty()
        );
    }

    #[tokio::test]
    async fn standalone_receiver_fix_keeps_the_expression_boundary() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path = std::env::temp_dir().join(format!("gin-root-style-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        let source = concat!(
            "Word is in 0 to 255\nBox has\n    value Word\n",
            "    read(self ref) Word: return self.value\n",
            "(Box.read(Box(value: 7)))\n",
        );
        backend
            .snapshot()
            .cache
            .set_contents(&path, source.to_owned());
        let actions = backend
            .handle_code_action(params(
                uri.clone(),
                Range::new(Position::new(4, 2), Position::new(4, 2)),
                vec![],
                Some(vec![CodeActionKind::QUICKFIX]),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actions.len(), 1, "{actions:?}");
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("code action");
        };
        let edits = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "(Box(value: 7).read)");
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(4, 0), Position::new(4, 25))
        );
    }

    #[tokio::test]
    async fn single_expression_value_body_offers_direct_binding_and_fix_all() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path =
            std::env::temp_dir().join(format!("gin-direct-bind-style-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        backend.snapshot().cache.set_contents(
            &path,
            "Word is in 0 to 255\nvalue Word:\n    1\n".to_owned(),
        );
        let params = params(
            uri.clone(),
            Range::new(Position::new(1, 11), Position::new(1, 11)),
            vec![],
            None,
        );
        let actions = backend.handle_code_action(params).await.unwrap().unwrap();
        assert_eq!(actions.len(), 2, "{actions:?}");
        let mut kinds = Vec::new();
        for action in actions {
            let CodeActionOrCommand::CodeAction(action) = action else {
                panic!("code action")
            };
            kinds.push(action.kind.unwrap());
            assert_eq!(action.is_preferred, Some(true));
            assert_eq!(
                action.edit.unwrap().changes.unwrap()[&uri],
                [TextEdit::new(
                    Range::new(Position::new(1, 11), Position::new(2, 5)),
                    " 1".to_owned(),
                )]
            );
        }
        assert!(kinds.contains(&CodeActionKind::QUICKFIX));
        assert!(kinds.contains(&CodeActionKind::new("source.fixAll.ginfmt")));
    }

    #[tokio::test]
    async fn unused_generic_type_name_help_removes_only_the_annotation() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path =
            std::env::temp_dir().join(format!("gin-generic-name-style-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        backend
            .snapshot()
            .cache
            .set_contents(&path, "release(a mut alloc): return a.release\n".to_owned());
        let params = params(
            uri.clone(),
            Range::new(Position::new(0, 15), Position::new(0, 15)),
            vec![],
            None,
        );
        let actions = backend.style_code_actions(&params);
        assert_eq!(actions.len(), 2, "{actions:?}");
        for action in actions {
            let CodeActionOrCommand::CodeAction(action) = action else {
                panic!("code action")
            };
            let edits = &action.edit.unwrap().changes.unwrap()[&uri];
            assert_eq!(
                edits,
                &[TextEdit::new(
                    Range::new(Position::new(0, 13), Position::new(0, 19)),
                    String::new(),
                )]
            );
        }
    }

    #[tokio::test]
    async fn parameter_name_help_renames_the_declaration_and_uses_together() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path =
            std::env::temp_dir().join(format!("gin-parameter-style-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        let source = "Allocation is in 0 to 255\nkeep(allocation Allocation) Allocation: return allocation\n";
        backend
            .snapshot()
            .cache
            .set_contents(&path, source.to_owned());
        let params = params(
            uri.clone(),
            Range::new(Position::new(1, 6), Position::new(1, 6)),
            vec![],
            None,
        );
        let actions = backend.style_code_actions(&params);
        assert_eq!(actions.len(), 2);
        for action in actions {
            let CodeActionOrCommand::CodeAction(action) = action else {
                panic!("code action")
            };
            let edits = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri];
            assert_eq!(edits.len(), 2);
            assert!(edits.iter().all(|edit| edit.new_text == "a"));
            assert_eq!(
                edits[0].range,
                Range::new(Position::new(1, 5), Position::new(1, 15))
            );
            assert_eq!(
                edits[1].range,
                Range::new(Position::new(1, 47), Position::new(1, 57))
            );
        }
    }

    #[tokio::test]
    async fn session_parameter_name_help_offers_a_rename_code_action() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path = std::env::temp_dir().join(format!(
            "gin-session-parameter-style-{}.gin",
            std::process::id()
        ));
        let uri = Url::from_file_path(&path).unwrap();
        let source = "Session is Open or Closed\nExitStatus is Success or Failure\nrun(session Session) ExitStatus:\n    terminal: session\n    return Failure\n";
        backend
            .snapshot()
            .cache
            .set_contents(&path, source.to_owned());
        let params = params(
            uri.clone(),
            Range::new(Position::new(2, 5), Position::new(2, 5)),
            vec![],
            Some(vec![CodeActionKind::QUICKFIX]),
        );
        let actions = backend.style_code_actions(&params);
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("code action")
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        let edits = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri];
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit.new_text == "s"));
        assert_eq!(
            edits[0].range,
            Range::new(Position::new(2, 4), Position::new(2, 11))
        );
        assert_eq!(
            edits[1].range,
            Range::new(Position::new(3, 14), Position::new(3, 21))
        );
    }

    #[tokio::test]
    async fn style_actions_use_unsaved_source_and_offer_fix_all_without_diagnostics() {
        let (service, _socket) = LspService::new(Backend::new);
        let backend = service.inner();
        let path =
            std::env::temp_dir().join(format!("gin-style-actions-{}.gin", std::process::id()));
        let uri = Url::from_file_path(&path).unwrap();
        let source = "Word is in 0 to 255\nBox has\n    value Word\n    read(self ref) Word: return self.value\nrun(box ref Box):\n    -- 🌍\n    value: Box.read(box)\n    return\n";
        backend
            .snapshot()
            .cache
            .set_contents(&path, source.to_owned());
        let params = params(
            uri.clone(),
            Range::new(Position::new(6, 15), Position::new(6, 15)),
            vec![],
            None,
        );
        let actions = backend.style_code_actions(&params);
        assert_eq!(actions.len(), 2);
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("code action")
        };
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        let edit = &action.edit.as_ref().unwrap().changes.as_ref().unwrap()[&uri][0];
        assert_eq!(edit.new_text, "box.read");
        assert_eq!(edit.range.start, Position::new(6, 11));
        let mut params = params;
        params.context.only = Some(vec![CodeActionKind::SOURCE_FIX_ALL]);
        let actions = backend.style_code_actions(&params);
        assert_eq!(actions.len(), 1);
        let CodeActionOrCommand::CodeAction(action) = &actions[0] else {
            panic!("code action")
        };
        assert_eq!(
            action.kind.as_ref().unwrap().as_str(),
            "source.fixAll.ginfmt"
        );
        params.context.only = Some(vec![CodeActionKind::REFACTOR]);
        assert!(backend.style_code_actions(&params).is_empty());
    }
}
