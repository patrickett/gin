use super::*;
use test_fixtures::TempPackage;

#[test]
fn use_completion_dep_root_nested_packages() {
    let pkg = TempPackage::new("dep_root_exports");
    pkg.write_flask_with_deps(
        "root",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "dep");
    pkg.write("dep/io/socket.gin", "connect := 1\n");
    pkg.write("dep/math/add.gin", "add := 1\n");
    pkg.write("main.gin", "use dep.\n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use dep.\n";
    let pos = Position {
        line: 0,
        character: "use dep.".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"io"));
    assert!(labels.contains(&"math"));
}

#[test]
fn use_completion_chained_nested_packages_under_dep_a() {
    let pkg = TempPackage::new("dep_a_exports");
    pkg.write_flask_with_deps(
        "root",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "dep");
    pkg.write("dep/a/root.gin", "a_root := 1\n");
    pkg.write("dep/a/b/one.gin", "b_fn := 1\n");
    pkg.write("dep/a/c/two.gin", "c_fn := 1\n");
    pkg.write("main.gin", "use dep.a.\n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use dep.a.\n";
    let pos = Position {
        line: 0,
        character: "use dep.a.".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"b"));
    assert!(labels.contains(&"c"));
}

#[test]
fn use_completion_filters_nested_names_by_partial() {
    let pkg = TempPackage::new("dep_root_exports_partial");
    pkg.write_flask_with_deps(
        "root",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "dep");
    pkg.write("dep/io/socket.gin", "connect := 1\n");
    pkg.write("dep/math/add.gin", "add := 1\n");
    pkg.write("main.gin", "use dep.i\n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use dep.i\n";
    let pos = Position {
        line: 0,
        character: "use dep.i".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"io"));
    assert!(!labels.contains(&"math"));
}

#[test]
fn use_completion_without_dot_completes_dependency_names() {
    let pkg = TempPackage::new("dep_names");
    pkg.write_flask_with_deps(
        "root",
        r#"core: "./core"
dep: "./dep"
"#,
    );
    pkg.write("main.gin", "use c\n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use c\n";
    let pos = Position {
        line: 0,
        character: "use c".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    // Dependency name completion is currently unfiltered; ensure the deps appear.
    assert!(labels.contains(&"core"));
    assert!(labels.contains(&"dep"));
}

#[test]
fn use_completion_package_member_import() {
    let pkg = TempPackage::new("member_import");
    pkg.write_flask_with_deps(
        "root",
        r#"core: "./core"
"#,
    );
    pkg.write_nested_flask("core", "core");
    pkg.write("core/int.gin", "Int is Unit\n");
    pkg.write("main.gin", "use core \n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use core \n";
    let pos = Position {
        line: 0,
        character: "use core ".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"Int"),
        "expected Int in completions after `use core `, got {:?}",
        labels
    );
}

#[test]
fn use_completion_shows_public_symbols() {
    let pkg = TempPackage::new("public_syms");
    pkg.write_flask_with_deps(
        "root",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "dep");
    pkg.write(
        "dep/io.gin",
        "println(s Str): return s\nprint(s Str): return s\n",
    );
    pkg.write("dep/maybe.gin", "Maybe is Some(value) or None\n");
    pkg.write("main.gin", "use dep \n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use dep \n";
    let pos = Position {
        line: 0,
        character: "use dep ".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"println"),
        "println not in labels: {:?}",
        labels
    );
    assert!(
        labels.contains(&"print"),
        "print not in labels: {:?}",
        labels
    );
    assert!(
        labels.contains(&"Maybe"),
        "Maybe not in labels: {:?}",
        labels
    );
    for item in &items {
        if item.label == "println" || item.label == "print" || item.label == "Maybe" {
            assert_eq!(
                item.kind,
                Some(CompletionItemKind::FUNCTION),
                "expected FUNCTION kind for {}",
                item.label
            );
        }
    }
}

#[test]
fn use_completion_filters_public_symbols_by_partial() {
    let pkg = TempPackage::new("public_syms_partial");
    pkg.write_flask_with_deps(
        "root",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "dep");
    pkg.write(
        "dep/io.gin",
        "println(s Str): return s\nprint(s Str): return s\n",
    );
    pkg.write("dep/maybe.gin", "Maybe is Some(value) or None\n");
    pkg.write("main.gin", "use dep p\n");

    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let src = "use dep p\n";
    let pos = Position {
        line: 0,
        character: "use dep p".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"println"));
    assert!(labels.contains(&"print"));
    assert!(!labels.contains(&"Maybe"));
}

#[test]
fn source_first_import_completion_selects_exports_left_to_right() {
    let pkg = TempPackage::new("source_first");
    pkg.write_flask_with_deps(
        "sample",
        r#"calc: "./math"
"#,
    );
    pkg.write("main.gin", "");
    pkg.write("math/ops.gin", "add := 1\nsub := 2\n");
    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    for (src, line, col, expected) in [
        ("use './math' ", 0, 13, vec!["add", "sub"]),
        ("use calc ", 0, 9, vec!["add", "sub"]),
        ("use './math' add, s", 0, 19, vec!["sub"]),
        ("use './math'\n    add\n    s", 2, 5, vec!["sub"]),
    ] {
        let position = Position::new(line, col);
        let items = Backend::use_completions(src, position, &uri, None).unwrap();
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        for label in expected {
            assert!(labels.contains(&label), "{src}: {labels:?}");
        }
        if line > 0 || col > 13 {
            assert!(!labels.contains(&"add"));
        }
        let item = items.iter().find(|item| item.label == "sub").unwrap();
        let Some(tower_lsp::lsp_types::CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("expected export replacement");
        };
        let start = src
            .position_to_byte_offset(edit.range.start.line, edit.range.start.character)
            .unwrap();
        let end = src
            .position_to_byte_offset(edit.range.end.line, edit.range.end.character)
            .unwrap();
        let completed = format!("{}{}{}", &src[..start], edit.new_text, &src[end..]);
        let parsed = parser::query::SourceParseExt::parse_source_full(completed.as_str());
        assert!(
            parsed.symptoms.is_empty(),
            "{completed}: {:?}",
            parsed.symptoms
        );
    }
    assert!(
        Backend::use_completions(
            "use './math' add\nvalue := 1",
            Position::new(1, 10),
            &uri,
            None
        )
        .is_none()
    );
}

#[test]
fn dotted_source_completion_only_suggests_modules() {
    let pkg = TempPackage::new("module_only");
    pkg.write_flask_with_deps(
        "app",
        r#"dep: "./dep"
"#,
    );
    pkg.write_nested_flask("dep", "library");
    pkg.write("dep/ops.gin", "add: 1\n");
    pkg.write("dep/math/ops.gin", "sub: 1\n");
    let uri = Url::from_file_path(pkg.join("main.gin")).unwrap();
    let items = Backend::use_completions("use dep.", Position::new(0, 8), &uri, None).unwrap();
    assert!(items.iter().any(|item| item.label == "math"));
    assert!(
        items
            .iter()
            .all(|item| item.kind == Some(CompletionItemKind::MODULE))
    );
}
