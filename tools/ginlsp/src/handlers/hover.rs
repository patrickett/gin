use crate::Backend;
#[cfg(test)]
use ast::DeclareValue;
use database::semantic_queries::hover_markdown;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use typeck::{
    byte_offset_to_position, get_char_at_position, get_number_at_position,
    get_range_literal_at_position, get_string_literal_at, is_in_comment, position_to_byte_offset,
    word_at_byte_offset,
};

impl Backend {
    pub(crate) async fn handle_hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        if self.is_shutdown() {
            return Ok(None);
        }

        let uri = params
            .text_document_position_params
            .text_document
            .uri
            .clone();
        let position = params.text_document_position_params.position;

        // Snapshot source + file id, drop DashMap ref before any spawn_blocking
        // await: `Ref` holds a shard read-lock and is `!Send`.
        let (source, file) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file),
            None => return Ok(None),
        };

        if is_in_comment(&source, position.line, position.character) {
            return Ok(None);
        }

        if let Some('(' | ')' | '[' | ']') =
            get_char_at_position(&source, position.line, position.character)
        {
            return Ok(None);
        }

        let Some(byte_pos) = position_to_byte_offset(&source, position.line, position.character)
        else {
            return Ok(None);
        };

        let dot_hover_range = compute_dot_hover_range(&source, byte_pos);

        if let Some(range_lit) =
            get_range_literal_at_position(&source, position.line, position.character)
        {
            if dot_hover_range.is_some() {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: String::from(
                            "```gin\n...\n```\n\n---\n\n\
                            Creates a `core.range.Range` from `start...end`.\n\n\
                            - `start`: lower bound\n\
                            - `end`: upper bound\n\n\
                            Example:\n\n\
                            ```gin\n\
                            r Range(Int) := 12...1200\n\
                            ```",
                        ),
                    }),
                    range: dot_hover_range,
                }));
            }
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```gin\n{range_lit}\n```"),
                }),
                range: dot_hover_range,
            }));
        }

        if let Some(num) = get_number_at_position(&source, position.line, position.character) {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```gin\n{num}\n```"),
                }),
                range: None,
            }));
        }

        if let Some(info) = get_string_literal_at(&source, byte_pos) {
            let (start_line, start_char) = byte_offset_to_position(info.range.start, &source);
            let (end_line, end_char) = byte_offset_to_position(info.range.end, &source);
            let range = Range {
                start: Position {
                    line: start_line,
                    character: start_char,
                },
                end: Position {
                    line: end_line,
                    character: end_char,
                },
            };
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("```gin\nvalue of literal: '{}'\n```", info.content),
                }),
                range: Some(range),
            }));
        }

        if let Some(word) = word_at_byte_offset(&source, byte_pos) {
            if word == "use" {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: String::from(
                            "```gin\nuse\n```\n\n\
                            ---\n\n\
                            Import items from other modules.\n\n\
                            **Local imports** — single-quoted path relative to the current file:\n\n\
                            ```gin\n\
                            use './math' as math\n\
                            use '../shared/utils'\n\
                            ```\n\n\
                            The path resolves to a module folder. All `.gin` files inside are \
                            included. Do not include the `.gin` extension.\n\n\
                            **Package imports** — dotted name from `flask.jsonc` dependencies:\n\n\
                            ```gin\n\
                            use core.io\n\
                            use http.web as web\n\
                            ```\n\n\
                            The root name (`core`, `http`) must be declared in `flask.jsonc` \
                            under `dependencies`. Subsequent segments resolve to files within \
                            the dependency directory.",
                        ),
                    }),
                    range: None,
                }));
            }
        }

        let import_hover = self
            .run_blocking_request("hover_import", move |this| {
                let snapshot = this.snapshot();
                let output = database::file_parse_output(&snapshot.db, file);
                let ast = &output.ast;

                match resolve::resolve_import_at(ast, &source, byte_pos) {
                    Some(resolve::ImportTarget::DepRoot { dep_name }) => {
                        hover_for_dependency_root(&uri, &dep_name)
                    }
                    Some(resolve::ImportTarget::DepSymbol { dep_name, symbol })
                    | Some(resolve::ImportTarget::BodySymbol { dep_name, symbol }) => {
                        resolve_import_hover(&uri, &dep_name, &symbol)
                    }
                    None => None,
                }
            })
            .await;

        if let Some(hover_text) = import_hover.flatten() {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: hover_text,
                }),
                range: None,
            }));
        }

        // General hover hits hover_markdown, which runs the parser via Salsa.
        // Off-load to the blocking pool so a stuck parse cannot pin the runtime.
        let Some(byte_pos_u32) = u32::try_from(byte_pos).ok() else {
            return Ok(None);
        };
        let value = self
            .run_blocking_request("hover", move |this| {
                let snapshot = this.snapshot();
                let markdown = hover_markdown(&snapshot.db, file, byte_pos_u32)?;

                // Check if this is a variant hover — if so, prepend the
                // package-qualified path (e.g. `core.Maybe` instead of `Maybe`).
                let output = database::file_parse_output(&snapshot.db, file);
                let source = file.contents(&snapshot.db);
                if let Some((_, parent_tag)) =
                    typeck::is_variant_at(source.as_str(), &output.ast, byte_pos)
                {
                    if let Some(qualifier) = package_name_for_file(&file.path(&snapshot.db)) {
                        let qualified = format!("{qualifier}.{parent_tag}");
                        let modified = markdown.replacen(
                            &format!("\n{parent_tag}\n"),
                            &format!("\n{qualified}\n"),
                            1,
                        );
                        return Some(modified);
                    }
                }

                Some(markdown)
            })
            .await
            .flatten();

        Ok(value.map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))
    }
}

/// Resolve an imported symbol from a dependency and return its hover text
/// from the definition file.
fn resolve_import_hover(uri: &Url, dep_name: &str, symbol: &str) -> Option<String> {
    let base_path = uri.to_file_path().ok()?;
    resolve::resolve_symbol_hover(&base_path, dep_name, symbol)
}

/// Resolve the dependency directory for `dep_name` from the `flask.jsonc`
/// found relative to the file at `uri`.
fn resolve_dep_dir_from_uri(uri: &Url, dep_name: &str) -> Option<std::path::PathBuf> {
    let base_path = uri.to_file_path().ok()?;
    resolve::resolve_dep_dir(&base_path, dep_name)
}

/// Read the package name from `flask.jsonc` for the package containing `file_path`.
fn package_name_for_file(file_path: &std::path::Path) -> Option<String> {
    let dir = file_path.parent()?;
    let config = flask::FlaskConfig::from_directory(dir)?;
    Some(config.name().to_string())
}

/// Show hover information about a dependency root (e.g. hovering over `core`
/// in `use core.true`).
fn hover_for_dependency_root(uri: &Url, dep_name: &str) -> Option<String> {
    let dep_dir = resolve_dep_dir_from_uri(uri, dep_name)?;
    let dep_config = flask::FlaskConfig::from_directory(&dep_dir)?;
    let name = dep_config.name();
    let version = dep_config.version();
    let description = dep_config.description().unwrap_or("");

    let mut text = format!("```gin\n{name}\n```");
    if !description.is_empty() {
        text.push_str(&format!("\n\n---\n\n{description}"));
    }
    text.push_str(&format!("\n\n---\n\nversion = {version}"));
    Some(text)
}

fn compute_dot_hover_range(source: &str, byte_pos: usize) -> Option<Range> {
    let bytes = source.as_bytes();
    if bytes.get(byte_pos) != Some(&b'.') {
        return None;
    }

    let start = if byte_pos >= 2
        && bytes.get(byte_pos - 2) == Some(&b'.')
        && bytes.get(byte_pos - 1) == Some(&b'.')
    {
        Some(byte_pos - 2)
    } else if byte_pos >= 1
        && bytes.get(byte_pos - 1) == Some(&b'.')
        && bytes.get(byte_pos + 1) == Some(&b'.')
    {
        Some(byte_pos - 1)
    } else if bytes.get(byte_pos + 1) == Some(&b'.') && bytes.get(byte_pos + 2) == Some(&b'.') {
        Some(byte_pos)
    } else {
        None
    }?;

    let (start_line, start_char) = byte_offset_to_position(start, source);
    let (end_line, end_char) = byte_offset_to_position(start + 3, source);
    Some(Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    })
}

/// Determine which part of a dotted path the cursor is on.
///
/// `part = 0` → root (e.g. `core` in `core.true`)
/// `part = 1` → first segment (`true` in `core.true`)
#[cfg(test)]
fn part_index_in_dotted_path(span_text: &str, byte_in_span: usize) -> usize {
    resolve::part_index_in_dotted_path(span_text, byte_in_span)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_index_root() {
        // byte in "core" part, before any dot
        assert_eq!(part_index_in_dotted_path("core.true", 0), 0);
        assert_eq!(part_index_in_dotted_path("core.true", 3), 0);
    }

    #[test]
    fn part_index_first_segment() {
        // byte in "true" part, after the dot
        assert_eq!(part_index_in_dotted_path("core.true", 5), 1); // 't'
        assert_eq!(part_index_in_dotted_path("core.true", 8), 1); // 'e'
    }

    #[test]
    fn part_index_multi_segment() {
        // "a.b.c" → parts: 0:a, 1:b, 2:c
        assert_eq!(part_index_in_dotted_path("a.b.c", 0), 0); // 'a'
        assert_eq!(part_index_in_dotted_path("a.b.c", 1), 0); // 'a'
        assert_eq!(part_index_in_dotted_path("a.b.c", 2), 1); // '.', actually it's the dot so...
        assert_eq!(part_index_in_dotted_path("a.b.c", 3), 1); // 'b'
        assert_eq!(part_index_in_dotted_path("a.b.c", 4), 2); // 'c'
        assert_eq!(part_index_in_dotted_path("a.b.c", 5), 2); // '.', it's at the dot
        assert_eq!(part_index_in_dotted_path("a.b.c", 6), 2); // 'c'
    }

    #[test]
    fn part_index_edge_right_at_dot() {
        // byte right at the `.` character — still counts as previous part
        // because the loop breaks BEFORE the dot (i >= byte_in_span)
        assert_eq!(part_index_in_dotted_path("core.true", 4), 0); // at the dot
        assert_eq!(part_index_in_dotted_path("a.b.c", 2), 1); // 'b'
        assert_eq!(part_index_in_dotted_path("a.b.c", 5), 2); // past end
    }

    #[test]
    fn hover_dep_root_shows_metadata() {
        // This test creates a real flask.jsonc for a dependency and verifies
        // that hover_for_dependency_root returns formatted metadata.
        let dir = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("core")).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{
  "name": "core",
  "version": "1.0.0",
  "description": "Core types and utilities",
  "authors": []
}
"#,
        );
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        let result = hover_for_dependency_root(&uri, "core");
        assert!(
            result.is_some(),
            "expected Some hover text for dependency root"
        );

        let text = result.unwrap();
        assert!(text.contains("core"), "should contain dep name");
        assert!(text.contains("1.0.0"), "should contain version");
        assert!(
            text.contains("Core types and utilities"),
            "should contain description"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hover_dep_root_unknown_dep_returns_none() {
        let dir = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {}
}
"#,
        );
        write_file(&dir.join("main.gin"), "main:\n    return 0\n");

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
        // Dep "core" is not in dependencies → should return None
        let result = hover_for_dependency_root(&uri, "core");
        assert!(result.is_none(), "expected None for unknown dependency");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn unique_temp_dir() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        dir.push(format!("ginlsp_hover_test_{pid}_{nanos}"));
        dir
    }

    fn write_file(path: &std::path::Path, contents: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, contents).unwrap();
    }

    /// `resolve_import_hover` on `"true"` from `core` must produce exactly
    /// the same markdown text as hovering over `true` inside `bool.gin`.
    /// This simulates both the use-statement case (Phase 1) and the body
    /// usage case (Phase 2) via the shared helper.
    #[test]
    fn resolve_import_hover_matches_definition_hover() {
        let dir = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("core")).unwrap();

        write_file(
            &dir.join("flask.jsonc"),
            r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "core": { "path": "core" }
  }
}
"#,
        );
        write_file(
            &dir.join("core/flask.jsonc"),
            r#"{"name":"core","version":"0.0.0","authors":[]}"#,
        );
        // Use the same bool.gin content the user supplied
        write_file(
            &dir.join("core/bool.gin"),
            r#"Bool is True or False

false := Bool.False
true  := Bool.True
"#,
        );
        write_file(
            &dir.join("main.gin"),
            "use core.true\n\nmain:\n    true\nreturn\n",
        );

        let uri = Url::from_file_path(dir.join("main.gin")).unwrap();

        // Compute the expected hover from bool.gin's definition
        let bool_source = std::fs::read_to_string(dir.join("core/bool.gin")).unwrap();
        let bool_po = parser::parse_source_full(&bool_source);
        let true_def_byte: usize = bool_source.find("true  :=").unwrap();
        let expected = typeck::hover_at(&bool_source, &bool_po.ast, true_def_byte)
            .expect("must produce hover text from bool.gin");

        // resolve_import_hover is the shared helper used by both Phase 1
        // (cursor in `use core.true`) and Phase 2 (cursor on `true` in body)
        let actual =
            resolve_import_hover(&uri, "core", "true").expect("must resolve imported symbol hover");

        assert_eq!(
            actual, expected,
            "resolve_import_hover must produce text identical to \
             hovering over `true` inside bool.gin"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Parse `maybe.gin` source and test hover on the `Maybe` tag name.
    ///
    /// Hovering `Maybe` should show the full tag declaration, its doc comment,
    /// and size/align metadata.
    #[test]
    fn hover_maybe_tag() {
        let source = "Maybe[x] is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
        let po = parser::parse_source_full(source);

        // Hover at "Maybe"
        let maybe_byte = source.find("Maybe").unwrap();
        let hover = typeck::hover_at(source, &po.ast, maybe_byte)
            .expect("hover_at should return text for `Maybe`");

        // Should contain the declaration
        assert!(
            hover.contains("Maybe[x] is Some(x) or None"),
            "hover on Maybe should show the tag declaration, got: {hover}"
        );

        // Should contain the doc comment
        assert!(
            hover.contains("Used to represent values that may or may not be present."),
            "hover on Maybe should show the doc comment"
        );

        // Should contain size and align metadata
        assert!(hover.contains("size ="), "hover on Maybe should show size");
        assert!(
            hover.contains("align ="),
            "hover on Maybe should show align"
        );
    }

    /// Test that hovering over `Some` variant inside `Maybe` tag declaration
    /// shows the parent tag name, the variant shape, and the variant's doc comment.
    #[test]
    fn hover_some_variant() {
        let source = "Maybe[x] is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
        let po = parser::parse_source_full(source);

        // Hover at "Some"
        let some_byte = source.find("Some").unwrap();
        let hover = typeck::hover_at(source, &po.ast, some_byte)
            .expect("hover_at should return text for `Some`");

        eprintln!("hover on Some produces: {hover:?}");

        // Should reference the parent tag
        assert!(
            hover.contains("Maybe"),
            "hover on Some should show parent tag Maybe, got: {hover}"
        );

        // Should show the variant shape
        assert!(
            hover.contains("Some(x)"),
            "hover on Some should show the variant shape, got: {hover}"
        );

        // Should include the variant's doc comment
        assert!(
            hover.contains("Has some value"),
            "hover on Some should show the variant doc comment, got: {hover}"
        );
    }

    /// Test that hovering over `None` variant inside `Maybe` tag declaration
    /// shows the parent tag name, the variant shape, and the variant's doc comment.
    #[test]
    fn hover_none_variant() {
        let source = "Maybe[x] is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
        let po = parser::parse_source_full(source);

        // Hover at "None"
        let none_byte = source.find("None").unwrap();
        let hover = typeck::hover_at(source, &po.ast, none_byte)
            .expect("hover_at should return text for `None`");

        eprintln!("hover on None produces: {hover:?}");

        // Should reference the parent tag
        assert!(
            hover.contains("Maybe"),
            "hover on None should show parent tag Maybe, got: {hover}"
        );

        // Should show the variant shape
        assert!(
            hover.contains("None"),
            "hover on None should show the variant shape, got: {hover}"
        );

        // Should include the variant's doc comment
        assert!(
            hover.contains("Has no value"),
            "hover on None should show the variant doc comment, got: {hover}"
        );
    }

    /// Parse the `maybe.gin` and verify that the tag declaration's variants
    /// (`Some`, `None`) are accessible from the `Declare` value, with their
    /// correct doc comments attached.
    #[test]
    fn maybe_variants_have_doc_comments() {
        let source = "Maybe[x] is    --- Used to represent values that may or may not be present.\n    Some(x) or --- Has some value `x`\n    None       --- Has no value\n";
        let po = parser::parse_source_full(source);

        let declare = po
            .ast
            .tags()
            .get(&internment::Intern::<String>::from_ref("Maybe"))
            .expect("Maybe tag should exist");

        let variants = match declare.value() {
            DeclareValue::Union { variants } => variants,
            other => panic!("expected Union variants, got {other:?}"),
        };

        assert_eq!(variants.len(), 2, "Maybe should have 2 variants");

        // Check the "Some" variant
        let some_variant = &variants[0];
        let some_doc = match some_variant {
            ast::Variant::Local { doc_comment, .. } => doc_comment,
            ast::Variant::External(_) => panic!("expected Local variant"),
        };
        assert!(some_doc.is_some(), "Some variant should have a doc comment");
        assert_eq!(some_doc.as_ref().unwrap().0.as_str(), "Has some value `x`");

        // Check the "None" variant
        let none_variant = &variants[1];
        let none_doc = match none_variant {
            ast::Variant::Local { doc_comment, .. } => doc_comment,
            ast::Variant::External(_) => panic!("expected Local variant"),
        };
        assert!(none_doc.is_some(), "None variant should have a doc comment");
        assert_eq!(none_doc.as_ref().unwrap().0.as_str(), "Has no value");
    }

    /// Verify that hovering on variants in a multi-variant tag works correctly.
    ///
    /// The parser requires `or` immediately after the first variant shape,
    /// before any doc comment (same pattern as `Maybe[x] is Some(x) or`).
    #[test]
    fn hover_multi_variant_tag() {
        let source =
            "Color is\n    Red or --- Pure red\n    Green or --- Pure green\n    Blue --- Pure blue\n";
        let po = parser::parse_source_full(source);

        for (variant, doc) in [
            ("Red", "Pure red"),
            ("Green", "Pure green"),
            ("Blue", "Pure blue"),
        ] {
            let byte = source.find(variant).unwrap();
            let hover = typeck::hover_at(source, &po.ast, byte)
                .unwrap_or_else(|| panic!("hover_at should return text for `{variant}`"));

            assert!(
                hover.contains("Color"),
                "hover on {variant} should show parent tag, got: {hover}"
            );
            assert!(
                hover.contains(variant),
                "hover on {variant} should show variant name, got: {hover}"
            );
            assert!(
                hover.contains(doc),
                "hover on {variant} should show doc: {doc}, got: {hover}"
            );
        }
    }
}
