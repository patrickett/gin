//! A `when` over a union of payload-less variants lowers to MLIR.
//!
//! A variant is its tag, a 64-bit value derived from its own name, so a variant means the
//! same number in every module and where a sum is expected; a `when` is a chain of
//! compares and selects. Arms are computed together, so an arm that would run an
//! instruction is refused, never executed when not taken.
use std::path::Path;
use std::process::Command;

fn fixture(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, text).unwrap();
    (directory, path)
}

fn emit(path: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .arg("mlir")
        .arg(path)
        .output()
        .unwrap()
}

fn mlir(text: &str) -> String {
    let (_directory, path) = fixture(text);
    let output = emit(&path);
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

const BOOL: &str = "\
#unrestricted
Bool is True or False

to_number(flag Bool) in 0 to 9: when flag is
    True then 7
    False then 3

negate(flag Bool) Bool: when flag is
    True then Bool.False
    False then Bool.True
";

#[test]
fn a_when_over_a_union_is_a_compare_and_select() {
    let text = mlir(BOOL);
    assert!(
        text.contains("func.func @to_number(%arg0: i64) -> i64"),
        "{text}"
    );
    assert!(text.contains("arith.cmpi eq, %arg0"), "{text}");
    assert!(text.contains("arith.select"), "{text}");
}

#[test]
fn a_function_may_take_and_return_a_union() {
    let text = mlir(BOOL);
    assert!(
        text.contains("func.func @negate(%arg0: i64) -> i64"),
        "{text}"
    );
}

#[test]
fn the_same_variant_is_the_same_tag_everywhere() {
    let text = mlir(&format!(
        "{BOOL}\
         a() Bool: return Bool.True\n\
         b() Bool: return Bool.True\n"
    ));
    let constants = |function: &str| -> String {
        let start = text.find(&format!("func.func @{function}")).unwrap();
        let body = &text[start..];
        body[..body.find("\n  }").unwrap()]
            .lines()
            .filter(|line| line.contains("arith.constant"))
            .map(|line| line.split('=').nth(1).unwrap().trim().to_owned())
            .collect()
    };
    assert_eq!(constants("a"), constants("b"));
    assert!(!constants("a").is_empty(), "{text}");
}

#[test]
fn different_variants_are_different_tags() {
    let text = mlir(
        "#unrestricted\nBool is True or False\n\
         t() Bool: return Bool.True\nf() Bool: return Bool.False\n",
    );
    let tag = |function: &str| -> String {
        let start = text.find(&format!("func.func @{function}")).unwrap();
        let body = &text[start..];
        body.lines()
            .find(|line| line.contains("arith.constant"))
            .unwrap()
            .to_owned()
    };
    let (t, f) = (tag("t"), tag("f"));
    assert_ne!(t.split('=').nth(1), f.split('=').nth(1), "{text}");
}

/// A union with a payload is not a tag: it fails closed, never guessed.
#[test]
fn a_union_with_a_payload_does_not_lower() {
    let text = "\
Count has
    value in 0 to 9

#unrestricted
Outcome is Count or Failed

describe(outcome Outcome) in 0 to 9: when outcome is
    Count(value) then value
    Failed then 0
";
    let (_directory, path) = fixture(text);
    let output = emit(&path);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("func.func"));
}

#[test]
fn three_variants_chain_two_selects() {
    let text = mlir(
        "#unrestricted\nColor is Red or Blue or Green\n\
         number(color Color) in 0 to 2: when color is\n    Red then 0\n    Blue then 1\n    Green then 2\n",
    );
    assert_eq!(text.matches("arith.select").count(), 2, "{text}");
}

#[test]
fn a_method_on_a_union_lowers() {
    let text = mlir(
        "#unrestricted\nSymbol is Putchar or Labs\n\n\
         Symbol has\n    code(self) in 0 to 1: when self is\n        Putchar then 0\n        Labs then 1\n",
    );
    assert!(
        text.contains("func.func @Symbol.code") || text.contains("@\"Symbol.code\""),
        "{text}"
    );
}

/// Text exists only while compiling, so a local of it that nothing reads leaves no trace and
/// must not stop the function lowering.
#[test]
fn a_text_local_that_nothing_reads_does_not_stop_lowering() {
    let text = mlir("f() in 0 to 5:\n    x := 'a'\n    return 3\n");
    assert!(text.contains("func.func @f() -> i64"), "{text}");
    assert!(text.contains("arith.constant 3"), "{text}");
}

/// An ignored union is simply not read.
#[test]
fn a_union_parameter_that_is_ignored_lowers() {
    let text =
        mlir("#unrestricted\nBool is True or False\nignore(flag Bool) in 0 to 0: return 0\n");
    assert!(
        text.contains("func.func @ignore(%arg0: i64) -> i64"),
        "{text}"
    );
}
