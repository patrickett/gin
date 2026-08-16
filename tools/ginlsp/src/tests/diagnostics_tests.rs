use super::*;
use ast::source::SourceExt;
use std::fs;
use std::path::Path;
use tower_lsp::lsp_types::Url;

fn quickfix_kind(data: Option<&serde_json::Value>) -> Option<&str> {
    data?.as_object()?.get("gincQuickFix")?.as_str()
}

/// `UnknownSymbol` uses the same `add-import` quickfix as undefined binds.
#[test]
fn unknown_symbol_add_import_quickfix_when_symbol_found_in_package() {
    let root = std::env::temp_dir().join(format!("ginlsp_unknown_symbol_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("string")).unwrap();
    fs::create_dir_all(root.join("primitive")).unwrap();
    fs::write(
        root.join("flask.jsonc"),
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(
        root.join("string/string.gin"),
        "ToString has (to_string String)\n",
    )
    .unwrap();
    fs::write(
            root.join("primitive/bool.gin"),
            "Bool is True or False and\n    has ToString(to_string: when self then 'true' else 'false')\n",
        )
        .unwrap();

    let source = fs::read_to_string(root.join("primitive/bool.gin")).unwrap();
    let path = Path::new(&root).join("primitive/bool.gin");
    let diag = Diagnostic::new(
        "type-unknown-symbol",
        "unknown symbol `ToString`".to_string(),
    )
    .with_arg("name", "ToString")
    .at_span(span::Span::new(0, 0));

    let converter = DiagnosticConverter::new(source.clone(), LineIndex::new(&source));
    let data = converter.diagnostic_quickfix_data(&diag, Some(&path));
    let obj = data
        .as_ref()
        .and_then(|v| v.as_object())
        .expect("quickfix data");
    assert_eq!(
        obj.get("gincQuickFix").and_then(|v| v.as_str()),
        Some("add-import")
    );
    let suggestions = obj
        .get("suggestions")
        .and_then(|v| v.as_array())
        .expect("suggestions");
    assert!(
        suggestions.iter().any(|s| {
            s.get("useLine")
                .and_then(|v| v.as_str())
                .is_some_and(|line| line.contains("ToString") && line.contains("string"))
        }),
        "expected import line for ToString: {suggestions:?}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_symbol_add_import_merges_with_existing_primitive_import() {
    let root = std::env::temp_dir().join(format!("ginlsp_merge_import_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("string")).unwrap();
    fs::create_dir_all(root.join("primitive")).unwrap();
    fs::write(
        root.join("flask.jsonc"),
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(root.join("primitive/list.gin"), "List(x) is Unit\n").unwrap();
    fs::write(root.join("primitive/int.gin"), "Byte is Unit\n").unwrap();
    let source = "use '../primitive/'.List\n\nString has (bytes List(Byte))\n";
    fs::write(root.join("string/string.gin"), source).unwrap();

    let path = Path::new(&root).join("string/string.gin");
    let diag = Diagnostic::new("type-unknown-symbol", "unknown symbol `Byte`".to_string())
        .with_arg("name", "Byte")
        .at_span(span::Span::new(0, 0));

    let converter = DiagnosticConverter::new(source.to_string(), LineIndex::new(source));
    let data = converter.diagnostic_quickfix_data(&diag, Some(&path));
    let obj = data
        .as_ref()
        .and_then(|v| v.as_object())
        .expect("quickfix data");
    let suggestions = obj
        .get("suggestions")
        .and_then(|v| v.as_array())
        .expect("suggestions");
    let merged = suggestions
        .iter()
        .find(|s| {
            s.get("useLine")
                .and_then(|v| v.as_str())
                .is_some_and(|line| line.contains("Byte") && line.contains("List"))
        })
        .expect("merged suggestion");
    assert_eq!(
        merged.get("useLine").and_then(|v| v.as_str()),
        Some("use '../primitive/'.(Byte, List)")
    );
    assert_eq!(merged.get("replaceLine").and_then(|v| v.as_u64()), Some(0));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_symbol_add_import_merges_with_existing_sibling_import() {
    let root = std::env::temp_dir().join(format!("ginlsp_merge_sibling_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("marker")).unwrap();
    fs::write(
        root.join("flask.jsonc"),
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    )
    .unwrap();
    fs::write(
        root.join("marker/marker.gin"),
        "Marker has (infect Unit)\nInfection is Unit\n",
    )
    .unwrap();
    let source = "use Marker\n\nCopy is Marker(Unit)\n";
    fs::write(root.join("marker/copy.gin"), source).unwrap();

    let path = Path::new(&root).join("marker/copy.gin");
    let diag = Diagnostic::new(
        "type-unknown-symbol",
        "unknown symbol `Infection`".to_string(),
    )
    .with_arg("name", "Infection")
    .at_span(span::Span::new(0, 0));

    let converter = DiagnosticConverter::new(source.to_string(), LineIndex::new(source));
    let data = converter.diagnostic_quickfix_data(&diag, Some(&path));
    let obj = data
        .as_ref()
        .and_then(|v| v.as_object())
        .expect("quickfix data");
    let suggestions = obj
        .get("suggestions")
        .and_then(|v| v.as_array())
        .expect("suggestions");
    let merged = suggestions
        .iter()
        .find(|s| {
            s.get("useLine")
                .and_then(|v| v.as_str())
                .is_some_and(|line| line.contains("Infection") && line.contains("Marker"))
        })
        .expect("merged suggestion");
    assert_eq!(
        merged.get("useLine").and_then(|v| v.as_str()),
        Some("use Infection, Marker")
    );
    assert_eq!(merged.get("replaceLine").and_then(|v| v.as_u64()), Some(0));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unknown_symbol_lsp_range_uses_diagnostic_span() {
    let source = "use Marker\n\nCopy is Marker(AllFields)\n";
    let start = source.find("AllFields").unwrap();
    let end = start + "AllFields".len();
    let diag = Diagnostic::new(
        "type-unknown-symbol",
        "unknown symbol `AllFields`".to_string(),
    )
    .with_arg("name", "AllFields")
    .at_span(span::Span::new(start, end));
    assert_eq!(&source[diag.span.start()..diag.span.end()], "AllFields");
    let line_index = ast::source::LineIndex::new(source);
    let converter = DiagnosticConverter::new(source.to_string(), line_index);
    let lsp =
        converter.symptoms_to_diagnostics(&[diag], &Url::from_file_path("/x.gin").unwrap(), None);
    let (line, col) = source.byte_offset_to_position(start);
    assert_eq!(lsp[0].range.start.line, line);
    assert_eq!(lsp[0].range.start.character, col);
}

#[test]
fn unknown_symbol_replace_binding_when_did_you_mean() {
    let diag = Diagnostic::new(
        "type-unknown-symbol",
        "unknown symbol `foo`, did you mean `main`?",
    )
    .with_arg("name", "foo")
    .with_arg("suggested", "main")
    .at_span(span::Span::new(0, 0));
    let converter = DiagnosticConverter::new(String::new(), LineIndex::new(""));
    assert_eq!(
        quickfix_kind(converter.diagnostic_quickfix_data(&diag, None).as_ref()),
        Some("replace-binding")
    );
}

#[test]
fn unreachable_else_omit_quickfix() {
    let diag = Diagnostic::new("type-unreachable-else-arm", "`else` arm is unreachable")
        .at_span(span::Span::new(0, 0));
    let converter = DiagnosticConverter::new(String::new(), LineIndex::new(""));
    assert_eq!(
        quickfix_kind(converter.diagnostic_quickfix_data(&diag, None).as_ref()),
        Some("omit-unreachable-else")
    );
}

#[test]
fn indexed_diagnostics_produce_correct_ranges() {
    let source = "use core.true\n\nmain:\n    core.true\n    check(core.true)\nreturn\n";
    let symptoms = [
        Diagnostic::new("parse-custom", "unexpected token").at_span(span::Span::new(0, 3)),
        Diagnostic::new("type-unknown-symbol", "unknown symbol `Foo`")
            .with_arg("name", "Foo")
            .at_span(span::Span::new(18, 21)),
        Diagnostic::new(
            "parse-bind-name-not-snake-case",
            "name should be snake_case",
        )
        .with_arg("name", "CamelCase")
        .with_arg("suggested", "camel_case")
        .at_span(span::Span::new(37, 45)),
    ];
    let uri = Url::from_file_path("/tmp/test.gin").unwrap();
    let line_index = ast::source::LineIndex::new(source);
    let converter = DiagnosticConverter::new(source.to_string(), line_index);

    let indexed = converter.symptoms_to_diagnostics(&symptoms, &uri, None);

    // Verify it produces the correct number of diagnostics.
    assert_eq!(indexed.len(), 3);

    // Each diagnostic should have a valid range.
    for (i, diag) in indexed.iter().enumerate() {
        assert!(
            diag.range.start <= diag.range.end,
            "diagnostic {i} has invalid range: {:?}",
            diag.range
        );
        assert!(diag.severity.is_some(), "diagnostic {i} missing severity");
        assert!(diag.code.is_some(), "diagnostic {i} missing code");
    }
}
