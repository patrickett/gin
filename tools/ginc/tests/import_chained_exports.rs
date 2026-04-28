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
fn chained_exports_dep_a_b_resolves_to_file() {
    let dir = unique_temp_dir("dep_a_b_file");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Root config declares path dependency `dep`.
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

    // dep exports a -> folder-module dep/a
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "a": { "path": "a" }
  }
}
"#,
    );
    // dep/a exports b -> b.gin
    write_file(
        &dir.join("dep/a/flask.jsonc"),
        r#"
{
  "name": "dep_a",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "b": { "path": "b.gin" }
  }
}
"#,
    );
    write_file(&dir.join("dep/a/b.gin"), "x: 1\n");

    // Entry uses multi-segment package path.
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
fn chained_exports_dep_a_b_imports_folder_module_exports() {
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
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "a": { "path": "a" }
  }
}
"#,
    );
    write_file(
        &dir.join("dep/a/flask.jsonc"),
        r#"
{
  "name": "dep_a",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "b": { "path": "b" }
  }
}
"#,
    );
    write_file(
        &dir.join("dep/a/b/flask.jsonc"),
        r#"
{
  "name": "dep_ab",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "c": { "path": "c.gin" }
  }
}
"#,
    );
    write_file(&dir.join("dep/a/b/c.gin"), "y: 2\n");

    // `b` resolves to a folder-module; importing `dep.a.b` should import its exports (c.gin).
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
fn missing_export_target_path_is_a_fatal_import_flaw() {
    let dir = unique_temp_dir("missing_export_target");
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

    // Export points to a missing path.
    write_file(
        &dir.join("dep/flask.jsonc"),
        r#"
{
  "name": "dep",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "io": { "path": "does_not_exist.gin" }
  }
}
"#,
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
        "expected compilation to fail (missing export target path)"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn local_folder_module_chained_segment_qualifies_intermediate_folder_exports() {
    let dir = unique_temp_dir("local_folder_chain_qual");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Local folder-module `utils/` with a direct `io` export and a nested folder-module export `a`.
    write_file(
        &dir.join("utils/flask.jsonc"),
        r#"
{
  "name": "utils",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "io": { "path": "io.gin" },
    "a": { "path": "a" }
  }
}
"#,
    );
    write_file(&dir.join("utils/io.gin"), "x: 1\n");

    // Intermediate folder-module `utils/a/` exporting its own `io`.
    write_file(
        &dir.join("utils/a/flask.jsonc"),
        r#"
{
  "name": "utils_a",
  "version": "0.0.0",
  "authors": [],
  "exports": {
    "io": { "path": "io.gin" }
  }
}
"#,
    );
    write_file(&dir.join("utils/a/io.gin"), "y: 2\n");

    // Import a direct file export and a chained folder-module export. If `use utils.a`
    // incorrectly qualifies as `utils.io`, it will conflict with `use utils.io`.
    write_file(
        &dir.join("main.gin"),
        r#"
use utils.io
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
        "expected compilation to succeed (utils.io and utils.a.io should not conflict)"
    );

    let _ = fs::remove_dir_all(&dir);
}

