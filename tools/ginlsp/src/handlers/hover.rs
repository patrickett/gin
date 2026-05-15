use crate::Backend;
#[cfg(test)]
use ast::DeclareValue;
use ast::Literal;

use ast::{byte_offset_to_position, get_char_at_position, is_in_comment, position_to_byte_offset};
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;

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

        // Snapshot source + file path, drop DashMap ref before any spawn_blocking
        // await: `Ref` holds a shard read-lock and is `!Send`.
        let (source, file_path) = match self.documents.get(&uri.to_string()) {
            Some(state) => (state.source.clone(), state.file_path.clone()),
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

        // Compute the word range at the cursor position for use in hover responses.
        let word_range = ast::word_byte_range(&source, byte_pos).map(|(start, end)| {
            let (sl, sc) = byte_offset_to_position(start, &source);
            let (el, ec) = byte_offset_to_position(end, &source);
            Range {
                start: Position {
                    line: sl,
                    character: sc,
                },
                end: Position {
                    line: el,
                    character: ec,
                },
            }
        });

        // Single blocking request for AST-based hover: range, number, string,
        // use-keyword, and import resolution.
        let fp = file_path.clone();
        let hover = self
            .run_blocking_request("hover", move |this| {
                let snapshot = this.snapshot();
                let output = snapshot.engine.parse_output(&fp)?;
                let ast = &output.ast;

                // Phase 1: literal detection via AST.
                if let Some((expr, sid)) = ast.expr_at_byte(byte_pos) {
                    match expr {
                        // Range literal: `start...end`
                        ast::Expr::Range(_) => {
                            if dot_hover_range.is_some() {
                                return Some(Hover {
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
                                });
                            }
                            let span = ast.span_table().get(sid);
                            let range_text = span.extract(&source);
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("```gin\n{range_text}\n```"),
                                }),
                                range: dot_hover_range,
                            });
                        }
                        // Numeric literal
                        ast::Expr::Lit(Literal::Number(n)) => {
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("```gin\n{n}\n```"),
                                }),
                                range: None,
                            });
                        }
                        ast::Expr::Lit(Literal::Int(i)) => {
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("```gin\n{i}\n```"),
                                }),
                                range: None,
                            });
                        }
                        ast::Expr::Lit(Literal::Float(f)) => {
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("```gin\n{f}\n```"),
                                }),
                                range: None,
                            });
                        }
                        // String literal: show content + highlighted range
                        ast::Expr::Lit(Literal::String(s)) => {
                            let span = ast.span_table().get(sid);
                            let (start_line, start_char) =
                                byte_offset_to_position(span.start, &source);
                            let (end_line, end_char) =
                                byte_offset_to_position(span.end, &source);
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
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("```gin\nvalue of literal: '{}'\n```", s),
                                }),
                                range: Some(range),
                            });
                        }
                        _ => {}
                    }
                }

                // Phase 2: "use" / "is" / "has" keyword hover.
                let word = ast.word_at_byte(byte_pos, &source)
                    .or_else(|| ast::word_at_byte_offset(&source, byte_pos));
                if word.as_deref() == Some("use") {
                    return Some(Hover {
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
                        range: word_range,
                    });
                }
                if word.as_deref() == Some("is") {
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: String::from(
                                "```gin\nis — defines a type alias\n```\n\n\
                                ---\n\n\
                                The left-hand name **is** shorthand for the right-hand shape.\n\n\
                                ```gin\n\
                                Pointer[x] is @x                     --- Pointer[x] = \u{27E8}pointer to x\u{27E9}\n\
                                String is (bytes List[Byte])          --- String = \u{27E8}list of bytes\u{27E9}\n\
                                ```\n\n\
                                #### Also used for\n\n\
                                **Unions** — the type is one of several variants:\n\n\
                                ```gin\n\
                                Bool is True or False\n\
                                Maybe[x] is Some(x) or None\n\
                                ```\n\n\
                                **Ranges** — the type is an integer within bounds:\n\n\
                                ```gin\n\
                                Int is 0...4294967295\n\
                                Byte is 0...255\n\
                                ```\n\n\
                                **Literal singletons** — a type with exactly one value:\n\n\
                                ```gin\n\
                                SysCallWrite is 4\n\
                                Nothing is ()\n\
                                ```\n\n\
                                > **Contrast with** `has`, which defines an interface \
                                (a type that other types implement).",
                            ),
                        }),
                        range: word_range,
                    });
                }
                if word.as_deref() == Some("has") {
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: String::from(
                                "```gin\nhas — defines an interface\n```\n\n\
                                ---\n\n\
                                The type describes what conforming types **have** — a set of \
                                named fields. Any type that provides those fields can implement \
                                this interface via an impl block.\n\n\
                                ```gin\n\
                                --- Define the interface\n\
                                ToString has (to_string String)\n\
                                ```\n\n\
                                ```gin\n\
                                --- Implement it on Bool\n\
                                Bool.ToString(to_string: when self then \u{27}true\u{27} else \u{27}false\u{27})\n\
                                ```\n\n\
                                #### More examples\n\n\
                                ```gin\n\
                                Register has (value Str)                        --- \u{27}has a value\u{27} interface\n\
                                List[x] has (pointer Pointer[x], length Length)  --- collection interface\n\
                                Range[x] has (start x, end x)                    --- bounded-range interface\n\
                                ```\n\n\
                                > **Contrast with** `is`, which defines a type alias \
                                (the name is shorthand for a shape).",
                            ),
                        }),
                        range: word_range,
                    });
                }

                // Phase 3: import resolution.
                match resolve::resolve_import_at(ast, &source, byte_pos) {
                    Some(resolve::ImportTarget::DepRoot { dep_name }) => {
                        return hover_for_dependency_root(&uri, &dep_name).map(|hover_text| Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: hover_text,
                            }),
                            range: word_range,
                        });
                    }
                    Some(resolve::ImportTarget::DepSymbol { dep_name, symbol })
                    | Some(resolve::ImportTarget::BodySymbol { dep_name, symbol }) => {
                        return resolve_import_hover(&uri, &dep_name, &symbol).map(
                            |hover_text| Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: hover_text,
                                }),
                                range: word_range,
                            },
                        );
                    }
                    Some(resolve::ImportTarget::LocalBundleSymbol {
                        local_path,
                        symbol,
                    }) => {
                        let base_path = uri.to_file_path().ok()?;
                        return resolve::resolve_local_symbol_hover(
                            &base_path,
                            &local_path,
                            &symbol,
                            &resolve::default_file_reader,
                        )
                        .map(|hover_text| Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: hover_text,
                            }),
                            range: word_range,
                        });
                    }
                    Some(resolve::ImportTarget::CurrentModuleSymbol { symbol }) => {
                        let base_path = uri.to_file_path().ok()?;
                        return resolve::resolve_current_module_hover(
                            &base_path,
                            &symbol,
                            &resolve::default_file_reader,
                        )
                        .map(|hover_text| Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: hover_text,
                            }),
                            range: word_range,
                        });
                    }
                    None => {}
                }

                None
            })
            .await;

        if let Some(Some(h)) = hover {
            return Ok(Some(h));
        }

        // General hover hits hover_markdown, which runs the parser via Salsa.
        // Off-load to the blocking pool so a stuck parse cannot pin the runtime.
        let Some(byte_pos_u32) = u32::try_from(byte_pos).ok() else {
            return Ok(None);
        };
        let file_path2 = file_path.clone();
        let value = self
            .run_blocking_request("hover_markdown", move |this| {
                let snapshot = this.snapshot();
                let markdown = snapshot.engine.hover(&file_path2, byte_pos_u32)?;

                // Check if this is a variant hover — if so, prepend the
                // package-qualified path (e.g. `core.Maybe` instead of `Maybe`).
                let (_source, output) = snapshot.engine.source_and_parse(&file_path2)?;
                if let Some((_, parent_tag)) = ast::hover::is_variant_at(&output.ast, byte_pos) {
                    if let Some(qualifier) = package_name_for_file(&file_path) {
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
            range: word_range,
        }))
    }
}

/// Resolve an imported symbol from a dependency and return its hover text
/// from the definition file.
fn resolve_import_hover(uri: &Url, dep_name: &str, symbol: &str) -> Option<String> {
    let base_path = uri.to_file_path().ok()?;
    resolve::resolve_symbol_hover(&base_path, dep_name, symbol, &resolve::default_file_reader)
}

/// Read the package name from `flask.jsonc` for the package containing `file_path`.
fn package_name_for_file(file_path: &std::path::Path) -> Option<String> {
    let dir = file_path.parent()?;
    let config = flask::FlaskConfig::from_directory(dir)?;
    Some(config.name().to_string())
}

/// Show hover information about a dependency root (e.g. hovering over `core`
/// in `use core.true`). Delegates to the shared resolver.
fn hover_for_dependency_root(uri: &Url, dep_name: &str) -> Option<String> {
    let base_path = uri.to_file_path().ok()?;
    resolve::resolve_dep_hover(&base_path, dep_name)
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
        let bool_analysis = ast::resolve_types(&bool_po.ast, std::slice::from_ref(&bool_po.ast));
        let expected =
            ast::hover::hover_at(&bool_source, &bool_po.ast, &bool_analysis, true_def_byte)
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
        let analysis = ast::resolve_types(&po.ast, std::slice::from_ref(&po.ast));
        let hover = ast::hover::hover_at(source, &po.ast, &analysis, maybe_byte)
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
        let analysis = ast::resolve_types(&po.ast, std::slice::from_ref(&po.ast));
        let hover = ast::hover::hover_at(source, &po.ast, &analysis, some_byte)
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
        let analysis = ast::resolve_types(&po.ast, std::slice::from_ref(&po.ast));
        let hover = ast::hover::hover_at(source, &po.ast, &analysis, none_byte)
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
        assert_eq!(
            some_doc.as_ref().unwrap().value.as_str(),
            "Has some value `x`"
        );

        // Check the "None" variant
        let none_variant = &variants[1];
        let none_doc = match none_variant {
            ast::Variant::Local { doc_comment, .. } => doc_comment,
            ast::Variant::External(_) => panic!("expected Local variant"),
        };
        assert!(none_doc.is_some(), "None variant should have a doc comment");
        assert_eq!(none_doc.as_ref().unwrap().value.as_str(), "Has no value");
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
            let analysis = ast::resolve_types(&po.ast, std::slice::from_ref(&po.ast));
            let hover = ast::hover::hover_at(source, &po.ast, &analysis, byte)
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
