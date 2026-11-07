//! Shared by the differential tests: run a program through the interpreter, and through the
//! compiler (`ginc --emit`, then `lli` or a native executable) to compare.
#![allow(dead_code)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use interpreter::{Error, Interpreter, Program, Value, with_big_stack};

/// The compiler binary the differential tests compare against: `$GINC`, else the debug
/// build in the workspace (`cargo build -p ginc`). `None` when it or the LLVM tools are absent.
pub fn compiler() -> Option<PathBuf> {
    let path = match std::env::var_os("GINC") {
        Some(path) => PathBuf::from(path),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ginc"),
    };
    let tools = ["mlir-opt", "mlir-translate", "lli"]
        .iter()
        .all(|tool| Command::new(tool).arg("--version").output().is_ok());
    (path.exists() && tools).then_some(path)
}

/// Whether a missing compiler is a failure (CI) or a skip (a laptop without LLVM).
pub fn require_compiled() -> bool {
    std::env::var_os("INTERPRETER_NEXT_REQUIRE_COMPILED").is_some()
}

/// `Some(compiler)` or, when it is absent and not required, `None` after saying so.
pub fn compiler_or_skip(test: &str) -> Option<PathBuf> {
    match compiler() {
        Some(path) => Some(path),
        None if require_compiled() => panic!("{test}: the compiler or the LLVM tools are missing"),
        None => {
            eprintln!("SKIPPED {test}: no ginc binary (cargo build -p ginc) or LLVM tools; interpreter-only checks ran");
            None
        }
    }
}

pub fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A one-module package directory around `main`.
pub fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    write(directory.path(), "mod.flask", "#settings\nmodule: \"diff\"\nversion: 0.1.0\n");
    write(directory.path(), "main.gin", main);
    directory
}

/// A library package `lib` (one file `util/util.gin`) and an application `app` depending on it.
pub fn two_packages(library: &str, app: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(root, "lib/mod.flask", "#settings\nmodule: \"lib\"\nversion: 0.1.0\n");
    write(root, "lib/util/util.gin", library);
    write(
        root,
        "app/mod.flask",
        "#settings\nmodule: \"app\"\nversion: 0.1.0\n\n#dependencies\nlib: \"../lib\"\n",
    );
    write(root, "app/main.gin", app);
    directory
}

/// What the interpreter says `function(n)` is, as an integer, for `n` in `0..=9`.
pub fn interpreted_inputs(source: &str, function: &str) -> Vec<Result<i128, Error>> {
    let source = source.to_owned();
    let function = function.to_owned();
    with_big_stack(move || {
        let program = Program::from_source(&source).expect("the program checks");
        (0..=9)
            .map(|n| {
                let mut interpreter = Interpreter::new(&program);
                interpreter
                    .call(&function, &[], &[Value::integer(n)])
                    .map(|outcome| outcome.value.as_integer().expect("an integer result").as_i128())
            })
            .collect()
    })
}

/// The interpreter's integer result of the `#entry` of a package.
pub fn interpreted_entry(root: &Path) -> Result<i128, Error> {
    let root = root.to_owned();
    with_big_stack(move || {
        let program = Program::from_package(&root).expect("the package checks");
        let mut interpreter = Interpreter::new(&program);
        interpreter
            .run_entry()
            .map(|outcome| outcome.value.as_integer().expect("an integer result").as_i128())
    })
}

fn pipe(program: &str, arguments: &[&str], input: &str) -> String {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{program}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

/// What the compiled `function(n)` returns for `n` in `0..=9`: the function is lowered by
/// `ginc --emit mlir`, wrapped in a `main` that takes `n` from the argument count so that
/// nothing is known at compile time, and run under `lli`.
pub fn compiled_inputs(ginc: &Path, source: &str, function: &str) -> Vec<i32> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, source).unwrap();
    let output = Command::new(ginc).arg("--emit").arg("mlir").arg(&path).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let start = text.find("module {").unwrap();
    let end = text.rfind("\n}").unwrap() + 2;
    let wrapper = text[start..end].replacen(
        "module {",
        &format!("module {{\n  func.func @main(%argc: i32) -> i32 {{\n    %one = arith.constant 1 : i32\n    %count = arith.subi %argc, %one : i32\n    %n = arith.extsi %count : i32 to i64\n    %r = func.call @{function}(%n) : (i64) -> i64\n    %out = arith.trunci %r : i64 to i32\n    return %out : i32\n  }}"),
        1,
    );
    let lowered = pipe(
        "mlir-opt",
        &[
            "--convert-scf-to-cf",
            "--convert-arith-to-llvm",
            "--convert-cf-to-llvm",
            "--convert-func-to-llvm",
            "--reconcile-unrealized-casts",
        ],
        &wrapper,
    );
    let llvm = pipe("mlir-translate", &["--mlir-to-llvmir"], &lowered);
    let file = directory.path().join("lowered.ll");
    std::fs::write(&file, llvm).unwrap();
    (0..=9)
        .map(|n| {
            Command::new("lli")
                .arg(&file)
                .args(vec!["x"; n])
                .status()
                .unwrap()
                .code()
                .expect("lli exits")
        })
        .collect()
}

/// The exit status of the executable `ginc --emit exe` builds for a package.
pub fn compiled_entry(ginc: &Path, root: &Path) -> Option<i32> {
    let scratch = tempfile::tempdir().unwrap();
    let program = scratch.path().join("program");
    let built = Command::new(ginc)
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(root)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    Command::new(&program).status().unwrap().code()
}
