use std::fs;
use std::path::PathBuf;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;

fn unique_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    dir.push(format!("gin_import_chain_{name}_{pid}_{nanos}"));
    dir
}

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

#[test]
fn chained_nested_dep_a_b_loads_package_at_b() {
    let dir = unique_temp_dir("dep_a_b_nested");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
    );

    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    write_file(
        &dir.join("dep/a/flask.jsonc"),
        r#"{"name":"dep_a","version":"0.0.0","authors":[]}"#,
    );
    write_file(
        &dir.join("dep/a/b/flask.jsonc"),
        r#"{"name":"dep_ab","version":"0.0.0","authors":[]}"#,
    );
    write_file(&dir.join("dep/a/b/x.gin"), "x: 1\n");

    write_file(
        &dir.join("main.gin"),
        "use dep.a.b\n\nmain:\n    return 0\n",
    );

    let exe = dir.join("out.exe");
    let mut args = Args {
        input: dir.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };
    GinCompiler::compile(&mut args);

    assert!(
        exe.exists(),
        "expected compilation to succeed and produce an executable"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn chained_exports_dep_a_b_imports_folder_module_sources() {
    let dir = unique_temp_dir("dep_a_b_folder");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
    );

    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    write_file(
        &dir.join("dep/a/flask.jsonc"),
        r#"{"name":"dep_a","version":"0.0.0","authors":[]}"#,
    );
    write_file(
        &dir.join("dep/a/b/flask.jsonc"),
        r#"{"name":"dep_ab","version":"0.0.0","authors":[]}"#,
    );
    write_file(&dir.join("dep/a/b/c.gin"), "y: 2\n");

    write_file(
        &dir.join("main.gin"),
        "use dep.a.b\n\nmain:\n    return 0\n",
    );

    let exe = dir.join("out.exe");
    let mut args = Args {
        input: dir.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };
    GinCompiler::compile(&mut args);

    assert!(
        exe.exists(),
        "expected compilation to succeed and produce an executable"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_nested_package_is_a_fatal_import_flaw() {
    let dir = unique_temp_dir("missing_nested");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "dep": { "path": "dep" }
  }
}
"#,
    );

    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    write_file(
        &dir.join("main.gin"),
        "use dep.io\n\nmain:\n    return 0\n",
    );

    let exe = dir.join("out.exe");
    let mut args = Args {
        input: dir.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };
    GinCompiler::compile(&mut args);

    assert!(
        !exe.exists(),
        "expected compilation to fail (no dep/io folder module)"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn chained_dep_utils_a_does_not_conflict_with_use_utils() {
    let dir = unique_temp_dir("utils_chain_qual");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("flask.jsonc"),
        r#"
{
  "name": "root",
  "version": "0.0.0",
  "authors": [],
  "dependencies": {
    "utils": { "path": "utils" }
  }
}
"#,
    );

    write_file(
        &dir.join("utils/flask.jsonc"),
        r#"{"name":"utils","version":"0.0.0","authors":[]}"#,
    );
    write_file(&dir.join("utils/io.gin"), "x: 1\n");

    write_file(
        &dir.join("utils/a/flask.jsonc"),
        r#"{"name":"utils_a","version":"0.0.0","authors":[]}"#,
    );
    write_file(&dir.join("utils/a/io.gin"), "y: 2\n");

    write_file(
        &dir.join("main.gin"),
        r#"
use utils
use utils.a

main:
    return 0
"#,
    );

    let exe = dir.join("out.exe");
    let mut args = Args {
        input: dir.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };
    GinCompiler::compile(&mut args);

    assert!(
        exe.exists(),
        "expected compilation: `use utils` (prefix utils.*) vs `use utils.a` (prefix a.*) do not conflict"
    );

    let _ = fs::remove_dir_all(&dir);
}
