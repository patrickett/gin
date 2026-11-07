//! Recorded compile-time queries and exact arithmetic lower to real artifacts: the MLIR carries
//! the number, and the native executable exits with it.
use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "mod.flask",
        "#settings\nmodule: \"consts\"\nversion: 0.1.0\n",
    );
    write(directory.path(), "main.gin", main);
    directory
}

fn ginc(arguments: &[&str], package: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(arguments)
        .arg(package)
        .output()
        .unwrap()
}

fn mlir(main: &str) -> String {
    let output = ginc(&["--emit", "mlir"], package(main).path());
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn run(main: &str) -> Option<i32> {
    let directory = package(main);
    let program = directory.path().join("program");
    let built = ginc(
        &["--emit", "exe", "--output", program.to_str().unwrap()],
        directory.path(),
    );
    assert!(built.status.success(), "{built:?}");
    Command::new(&program).status().unwrap().code()
}

const POINT: &str = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n\n";

#[test]
fn a_size_is_an_emitted_constant_and_the_executable_exits_with_it() {
    let text = mlir(&format!("{POINT}size() in 0 to 255: return #size(Point)\n"));
    assert!(text.contains("arith.constant 4 : i64"), "{text}");
    assert_eq!(
        run(&format!(
            "{POINT}#entry\nstart() in 0 to 255: return #size(Point)\n"
        )),
        Some(4)
    );
    assert_eq!(
        run(&format!(
            "{POINT}#entry\nstart() in 0 to 255: return #size(Point) * 10 + #alignment(Point)\n"
        )),
        Some(42)
    );
}

#[test]
fn the_target_register_width_is_emitted() {
    assert_eq!(
        run("#entry\nstart() in 0 to 255: return #target.register_bits\n"),
        Some(64)
    );
}

#[test]
fn a_size_that_depends_on_a_type_parameter_lowers_per_instance() {
    // The node becomes the instance's number: `Word` is one byte, `Pair` two.
    let source = "#unrestricted\nWord has value in 0 to 255\n#unrestricted\nPair has\n    a in 0 to 255\n    b in 0 to 255\n\nwidth(element) in 0 to 4611686018427387904: return #size(element)\n\n#entry\nstart() in 0 to 255:\n    x in 0 to 4611686018427387904: width(Word)\n    y in 0 to 4611686018427387904: width(Pair)\n    when y > x then\n        return 7\n    return 1\n";
    assert_eq!(run(source), Some(7));
    // Control: the same size twice is not greater.
    let same = source.replace("width(Pair)", "width(Word)");
    assert_eq!(run(&same), Some(1));
}
