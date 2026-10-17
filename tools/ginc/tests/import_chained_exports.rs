mod common;
use common::{compile_exe, require_success};
use std::fs;

use ginc::cli::{Args, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::{host_target_triple, unique_temp_dir, write_file};

#[test]
fn chained_nested_dep_a_b_loads_package_at_b() {
    let dir = unique_temp_dir("dep_a_b_nested");
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("mod.flask"),
        r#"#settings
module: "root"
version: 0.0.0

#dependencies
dep: "./dep"
"#,
    );

    write_file(
        &dir.join("dep/mod.flask"),
        r#"#settings
module: "dep"
version: 0.0.0

#dependencies
"#,
    );
    write_file(
        &dir.join("dep/a/mod.flask"),
        r#"#settings
module: "dep_a"
version: 0.0.0

#dependencies
"#,
    );
    write_file(
        &dir.join("dep/a/b/mod.flask"),
        r#"#settings
module: "dep_ab"
version: 0.0.0

#dependencies
"#,
    );
    write_file(&dir.join("dep/a/b/x.gin"), "x: 1\n");

    write_file(&dir.join("main.gin"), "use dep.a.b\n\n#entry\nmain: 0\n");

    let exe = dir.join("out.exe");
    let result = compile_exe(dir.join("main.gin"), &exe.clone(), Profile::Debug);
    require_success(&result, Profile::Debug);

    assert!(
        exe.exists(),
        "expected compilation to succeed and produce an executable"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn missing_nested_package_is_a_fatal_import_flaw() {
    let dir = unique_temp_dir("missing_nested");
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("mod.flask"),
        r#"#settings
module: "root"
version: 0.0.0

#dependencies
dep: "./dep"
"#,
    );

    write_file(
        &dir.join("dep/mod.flask"),
        r#"#settings
module: "dep"
version: 0.0.0

#dependencies
"#,
    );
    write_file(&dir.join("main.gin"), "use dep.io\n\n#entry\nmain: 0\n");

    let exe = dir.join("out.exe");
    compile_exe(dir.join("main.gin"), &exe, Profile::Debug);

    assert!(
        !exe.exists(),
        "expected compilation to fail (no dep/io folder module)"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn chained_dep_utils_a_does_not_conflict_with_use_utils() {
    let dir = unique_temp_dir("utils_chain_qual");
    fs::create_dir_all(&dir).unwrap();

    write_file(
        &dir.join("mod.flask"),
        r#"#settings
module: "root"
version: 0.0.0

#dependencies
utils: "./utils"
"#,
    );

    write_file(
        &dir.join("utils/mod.flask"),
        r#"#settings
module: "utils"
version: 0.0.0

#dependencies
"#,
    );
    write_file(&dir.join("utils/io.gin"), "x: 1\n");

    write_file(
        &dir.join("utils/a/mod.flask"),
        r#"#settings
module: "utils_a"
version: 0.0.0

#dependencies
"#,
    );
    write_file(&dir.join("utils/a/io.gin"), "y: 2\n");

    write_file(
        &dir.join("main.gin"),
        r#"
use utils
use utils.a

#entry
main: 0
"#,
    );

    let exe = dir.join("out.exe");
    let result = compile_exe(dir.join("main.gin"), &exe.clone(), Profile::Debug);
    require_success(&result, Profile::Debug);

    assert!(
        exe.exists(),
        "expected compilation: `use utils` (prefix utils.*) vs `use utils.a` (prefix a.*) do not conflict"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn nested_import_binds_its_module_name_for_calls() {
    let dir = unique_temp_dir("nested_module_call");
    write_file(
        &dir.join("mod.flask"),
        r#"#settings
module: "app"
version: 0.0.0

#dependencies
package: "./dep"
"#,
    );
    write_file(
        &dir.join("dep/mod.flask"),
        r#"#settings
module: "library"
version: 0.0.0

#dependencies
"#,
    );
    write_file(
        &dir.join("dep/math/ops.gin"),
        &format!("{}answer Word: 42\n", test_fixtures::preludes::WORD_PRELUDE),
    );
    for source in [
        "use package.math\n#entry\nmain: math.answer\n",
        "use package.math as numbers\n#entry\nmain: numbers.answer\n",
        "use package.math answer\n#entry\nmain: answer\n",
    ] {
        write_file(&dir.join("main.gin"), source);
        let exe = dir.join("out");
        let result = GinCompiler::compile(&mut Args {
            input: dir.join("main.gin"),
            output: Some(exe.clone()),
            target: Some(host_target_triple()),
            gin_home: Some(flask::GinHome::new(dir.join("gin-home"))),
            no_cache: true,
            ..Default::default()
        });
        assert!(result.is_success(), "{source}: {result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{source}: {result:?}");
        let status = std::process::Command::new(exe).status().unwrap();
        assert_eq!(status.code(), Some(42), "{source}");
    }
    fs::remove_dir_all(dir).unwrap();
}
