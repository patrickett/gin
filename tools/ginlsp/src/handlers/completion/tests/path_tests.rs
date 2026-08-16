use super::*;
use std::fs;
use std::path::PathBuf;

fn unique_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    dir.push(format!("ginlsp_use_complete_{name}_{pid}_{nanos}"));
    dir
}

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn use_completion_dep_root_nested_packages() {
    let dir = unique_temp_dir("dep_root_exports");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
    );
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
    );
    fs::create_dir_all(dir.join("dep/io")).unwrap();
    fs::create_dir_all(dir.join("dep/math")).unwrap();
    write_file(&dir.join("dep/io/socket.gin"), "connect := 1\n");
    write_file(&dir.join("dep/math/add.gin"), "add := 1\n");
    write_file(&dir.join("main.gin"), "use dep.\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use dep.\n";
    let pos = Position {
        line: 0,
        character: "use dep.".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"io"));
    assert!(labels.contains(&"math"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_chained_nested_packages_under_dep_a() {
    let dir = unique_temp_dir("dep_a_exports");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
    );
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
    );
    fs::create_dir_all(dir.join("dep/a/b")).unwrap();
    fs::create_dir_all(dir.join("dep/a/c")).unwrap();
    write_file(&dir.join("dep/a/root.gin"), "a_root := 1\n");
    write_file(&dir.join("dep/a/b/one.gin"), "b_fn := 1\n");
    write_file(&dir.join("dep/a/c/two.gin"), "c_fn := 1\n");
    write_file(&dir.join("main.gin"), "use dep.a.\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use dep.a.\n";
    let pos = Position {
        line: 0,
        character: "use dep.a.".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"b"));
    assert!(labels.contains(&"c"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_filters_nested_names_by_partial() {
    let dir = unique_temp_dir("dep_root_exports_partial");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
    );
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
    );
    fs::create_dir_all(dir.join("dep/io")).unwrap();
    fs::create_dir_all(dir.join("dep/math")).unwrap();
    write_file(&dir.join("dep/io/socket.gin"), "connect := 1\n");
    write_file(&dir.join("dep/math/add.gin"), "add := 1\n");
    write_file(&dir.join("main.gin"), "use dep.i\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use dep.i\n";
    let pos = Position {
        line: 0,
        character: "use dep.i".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"io"));
    assert!(!labels.contains(&"math"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_without_dot_completes_dependency_names() {
    let dir = unique_temp_dir("dep_names");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "core": { "path": "core" }, "dep": { "path": "dep" } }
}
"#,
    );
    write_file(&dir.join("main.gin"), "use c\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
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

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_package_member_import() {
    let dir = unique_temp_dir("member_import");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
    );
    write_file(
        &dir.join("core/flask.jsonc"),
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    write_file(&dir.join("core/int.gin"), "Int is Unit\n");
    write_file(&dir.join("main.gin"), "use core.\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use core.\n";
    let pos = Position {
        line: 0,
        character: "use core.".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"Int"),
        "expected Int in completions after `use core.`, got {:?}",
        labels
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_shows_public_symbols() {
    let dir = unique_temp_dir("public_syms");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
    );
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
    );
    write_file(
        &dir.join("dep/io.gin"),
        "println(s Str): s\nprint(s Str): s\n",
    );
    write_file(&dir.join("dep/maybe.gin"), "Maybe is Some(value) or None\n");
    write_file(&dir.join("main.gin"), "use dep.\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use dep.\n";
    let pos = Position {
        line: 0,
        character: "use dep.".len() as u32,
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

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn use_completion_filters_public_symbols_by_partial() {
    let dir = unique_temp_dir("public_syms_partial");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": { "dep": { "path": "dep" } }
}
"#,
    );
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": []
}
"#,
    );
    write_file(
        &dir.join("dep/io.gin"),
        "println(s Str): s\nprint(s Str): s\n",
    );
    write_file(&dir.join("dep/maybe.gin"), "Maybe is Some(value) or None\n");
    write_file(&dir.join("main.gin"), "use dep.p\n");

    let uri = Url::from_file_path(dir.join("main.gin")).unwrap();
    let src = "use dep.p\n";
    let pos = Position {
        line: 0,
        character: "use dep.p".len() as u32,
    };
    let items = Backend::use_completions(src, pos, &uri, None).unwrap();
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"println"));
    assert!(labels.contains(&"print"));
    assert!(!labels.contains(&"Maybe"));

    let _ = fs::remove_dir_all(&dir);
}
