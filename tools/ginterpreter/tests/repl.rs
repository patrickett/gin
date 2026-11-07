//! The REPL through standard input (a pipe, so no line editor), as a smoke test.
use std::io::Write;
use std::process::{Command, Stdio};

fn repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    (String::from_utf8(output.stdout).unwrap(), String::from_utf8(output.stderr).unwrap())
}

#[test]
fn declarations_expressions_flaws_holes_and_commands() {
    let (out, err) = repl(concat!(
        "five() in 0 to 9: return 5\n",
        "five() + 1\n",
        "$type five() + 1\n",
        "bad(n in 0 to 9) in 0 to 9: return n - 3\n",
        "$check\n",
        "h(n in 0 to 9) in 0 to 9:\n    when n < 5 then return 1\n    return #todo\n\n",
        "$holes\n",
        "$scope\n",
        "missing()\n",
        "#bogus\n\n",
        "$reset\n",
        "$scope\n",
        "$quit\n",
        "five()\n",
    ));
    assert!(out.contains("ok: added fn five/0"), "{out}");
    assert!(out.contains("6 : 6 to 6"), "{out}");
    assert!(out.lines().any(|l| l == "6 to 6"), "{out}");
    assert!(out.contains("rejected: 1 flaw(s); the session is unchanged"), "{out}");
    assert!(out.contains("ok: added fn h/1"), "{out}");
    // Positions are in the accumulated session source (`five` and the rejected text precede it).
    assert!(out.lines().any(|l| l.starts_with("hole in h at ") && l.ends_with(":12: expected 0 to 9")), "{out}");
    assert!(out.contains("known: 5 <= n <= 9"), "{out}");
    assert!(out.contains("function five(") && out.contains("function h("), "{out}");
    // `#bogus` is not a command, so it is the language's, and the language does not know it.
    assert!(err.contains("bogus"), "{err}");
    assert!(out.contains("session cleared"), "{out}");
    // Nothing after $quit runs.
    assert!(!out.contains("5 : "), "{out}");
    // Flaws are rendered the way `ginc --check-new` renders them, on standard error.
    assert!(err.contains("source-check-type"), "{err}");
    assert!(err.contains("source-check-unknown-name"), "{err}");
    assert!(err.contains("in `missing()`"), "{err}");
    assert!(!err.contains("__gin_eval"), "the wrapper is not shown: {err}");
    assert!(err.contains("source-check-hole"), "{err}");
}

#[test]
fn a_loaded_file_extends_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lib.gin");
    std::fs::write(&file, "two() in 0 to 9: return 2\n").unwrap();
    let (out, _) = repl(&format!("$load {}\ntwo() + 3\n", file.display()));
    assert!(out.contains("loaded 1 declaration(s)"), "{out}");
    assert!(out.contains("5 : 5 to 5"), "{out}");
}

#[test]
fn inspect_and_image_commands_show_shape_size_and_dependencies() {
    let (out, _) = repl(concat!(
        "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n\n",
        "base(n in 0 to 9) in 0 to 9: return n\n",
        "top(n in 0 to 9) in 0 to 9: return base(n)\n",
        "$inspect Point\n",
        "$image\n",
    ));
    assert!(out.contains("\"type_kind\": \"record\""), "{out}");
    assert!(out.contains("\"value\": 4"), "{out}");
    assert!(out.contains("\"fn top/1\""), "{out}");
}

#[test]
fn directives_in_expressions_stay_the_languages_and_commands_are_by_name() {
    let (out, err) = repl(concat!(
        "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n\n",
        "#size(Point)\n",
        "#alignment(Point) + 1\n",
        "$eval #size(Point)\n",
        "#entry\nstart() in 0 to 255: return 7\n\n",
        "start()\n",
        "$check\n",
        ":inspect Point\n",
    ));
    assert!(out.contains("4 : 4 to 4"), "{out}");
    assert!(out.contains("3 : 3 to 3"), "{out}");
    assert!(out.contains("7 : 7 to 7"), "a marker line still starts a declaration: {out}");
    assert!(out.contains("0 flaw(s), 0 warning(s)"), "{out}");
    // The `:` spelling is gone: it is not Gin, so it is a syntax error, not a command.
    assert!(err.contains("parse-"), "{err}");
    assert!(!out.contains("\"type_kind\""), "{out}");
}

#[test]
fn eval_and_check_are_explicit_forms() {
    let (out, _) = repl("five() in 0 to 9: return 5\n$eval five() + 1\n$type five()\n$check\n$help\n");
    assert!(out.contains("6 : 6 to 6"), "{out}");
    assert!(out.lines().any(|l| l == "5 to 5"), "{out}");
    assert!(out.contains("Commands start with `$`; `#` is the language's"), "{out}");
    assert!(!out.contains(" :inspect") && !out.contains("\n  :"), "{out}");
}
