//! A trait of another module: its default method is checked and lowered in the module whose
//! type provides it. The exit status of `start()` is what the program observes.
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn ginc(arguments: &[&str], package: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(arguments)
        .arg(package)
        .output()
        .unwrap()
}

fn two_packages(library: &str, app: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write(
        root,
        "lib/mod.flask",
        "#settings\nmodule: \"lib\"\nversion: 0.1.0\n",
    );
    write(root, "lib/util/util.gin", library);
    write(
        root,
        "app/mod.flask",
        "#settings\nmodule: \"app\"\nversion: 0.1.0\n\n#dependencies\nlib: \"../lib\"\n",
    );
    write(root, "app/main.gin", app);
    directory
}

const LIBRARY: &str = "Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b
";

#[test]
fn a_default_method_of_an_imported_trait_runs() {
    let app = "use lib.util Numeric\n\n#unrestricted\nCount is in 0 to 99\nCount has Numeric\n\n#entry\nstart() in 0 to 255:\n    a Count := 30\n    b Count := 12\n    sum in 0 to 198: a + b\n    return sum\n";
    let directory = two_packages(LIBRARY, app);
    let root = directory.path().join("app");
    let program = directory.path().join("program");
    let built = ginc(
        &["--emit", "exe", "--output", program.to_str().unwrap()],
        &root,
    );
    assert!(built.status.success(), "{built:?}");
    assert_eq!(Command::new(&program).status().unwrap().code(), Some(42));
}

#[test]
fn a_trait_the_app_did_not_import_is_refused() {
    let app = "#unrestricted\nCount is in 0 to 99\nCount has Numeric\n\n#entry\nstart() in 0 to 255: return 1\n";
    let directory = two_packages(LIBRARY, app);
    let output = ginc(&["--emit", "mlir"], &directory.path().join("app"));
    assert!(!output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("unknown-name"), "{text}");
}
