use std::io::Write;
use std::process::{Command, Output, Stdio};
use test_fixtures::TempPackage;

const NUMBERS: &str = r#"Signed64 is in -9223372036854775808 to 9223372036854775807
Signed64 has
    finish(self):
        return

signed64_add_bits(lhs Signed64, rhs Signed64) Signed64:
    left is in -9223372036854775808 to 9223372036854775807: lhs
    right is in -9223372036854775808 to 9223372036854775807: rhs
    biased: left + right + 9223372036854775808
    bits is in 0 to 18446744073709551615: biased & 18446744073709551615
    return (bits - 9223372036854775808) as Signed64
#infix('+')
signed64_add(lhs Signed64, rhs Signed64) Signed64: return signed64_add_bits(lhs, rhs)
"#;

fn run(package: &TempPackage, input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter"))
        .arg(package.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("interpreter process");
    child
        .stdin
        .take()
        .expect("interpreter stdin")
        .write_all(input.as_bytes())
        .expect("terminal submissions");
    child.wait_with_output().expect("interpreter output")
}

#[test]
fn terminal_holes_can_be_inspected_repaired_and_saved_for_replay() {
    use ginterpreter::{InterpreterContext, InterpreterSession, RuntimeStorage, Submission};

    let package = TempPackage::new("interpreter_terminal_holes");
    package.write_flask("interpreter_terminal_holes");
    package.write("numbers/defs.gin", NUMBERS);
    let exports = TempPackage::new("interpreter_terminal_exports");
    let blocked = exports.write("blocked.gin", "previous export");
    let saved = exports.path().join("saved.gin");
    let input = format!(
        "use 'numbers' Signed64, signed64_add_bits, signed64_add\n\
         unfinished Signed64: #todo\n#type (unfinished)\n#scope\n\
         #save {}\n(unfinished)\n#frames\n\
         unfinished Signed64: 42\n#retry\n#save {}\n#q\n",
        blocked.display(),
        saved.display(),
    );
    let output = run(&package, &input);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("unfilled-hole"), "{stderr}");
    assert!(stderr.contains("compile-time-unfilled-hole"), "{stderr}");
    assert!(
        stdout.contains("unfinished: unfinished Signed64"),
        "{stdout}"
    );
    assert!(stdout.contains("0: unfinished at"), "{stdout}");
    let answers: Vec<_> = stdout
        .lines()
        .map(|line| line.trim_start_matches(['>', ' ']))
        .collect();
    assert!(answers.contains(&"Signed64"), "{stdout}");
    assert!(answers.contains(&"42"), "{stdout}");
    assert_eq!(std::fs::read_to_string(blocked).unwrap(), "previous export");
    let source = std::fs::read_to_string(saved).unwrap();
    assert!(!source.contains("#todo"));
    assert!(!source.contains("__gin"));
    assert!(!stdout.contains("__gin"), "{stdout}");
    assert!(!stderr.contains("__gin"), "{stderr}");
    let context = InterpreterContext::load(package.path()).unwrap();
    let mut replay = InterpreterSession::with_context(context);
    let accepted = replay.submit(&source);
    assert!(
        matches!(accepted, Submission::DeclarationAccepted { .. }),
        "{accepted:?}"
    );
    let result = replay.submit("(unfinished)");
    assert!(matches!(result, Submission::Evaluated { value, .. }
        if value.storage == RuntimeStorage::Int64(42)));
    assert_eq!(replay.materialize().unwrap(), source);
}

#[test]
fn terminal_suggests_shared_code_actions_without_discarding_the_pending_condition() {
    let package = TempPackage::new("interpreter_terminal_actions");
    package.write_flask("interpreter_terminal_actions");
    package.write("numbers/defs.gin", NUMBERS);
    let output = run(
        &package,
        concat!(
            "use 'numbers' Signed64, signed64_add_bits, signed64_add\n",
            "value Signed64:\n    missing\n\n",
            "#suggest\n#abort\nvalue Signed64:\n    7\n\n#suggest\n#q\n",
        ),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert_eq!(
        stdout.matches("fmt-single-expression-body").count(),
        2,
        "{stdout}"
    );
    assert!(stdout.contains("bytes "), "{stdout}");
    assert!(stdout.contains("pending condition discarded"), "{stdout}");
    assert!(stderr.contains("type-unknown-symbol"), "{stderr}");
    assert!(!stdout.contains("__gin"), "{stdout}");
}

#[test]
fn terminal_exposes_trap_frames_and_abort() {
    let package = TempPackage::new("interpreter_terminal_frames");
    package.write_flask("interpreter_terminal_frames");
    package.write(
        "nested/defs.gin",
        concat!(
            "Word is in 0 to 255\nWord has\n    finish(self):\n        return\n",
            "inner(flag Word) Word:\n    local Word: 7\n    while flag is 1 do\n        iteration: flag\n    return local\n",
            "outer(flag Word) Word:\n    retained Word: 9\n    return inner(flag)\n",
        ),
    );
    let output = run(
        &package,
        concat!(
            "use 'nested' Word, inner, outer\n(outer(1))\n#frames\n#frame 0\n",
            "#abort\n#frames\n#q\n",
        ),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(stderr.contains("compile-time-fuel-exhausted"), "{stderr}");
    assert!(stdout.contains("0: inner at"), "{stdout}");
    assert!(stdout.contains("1: outer at"), "{stdout}");
    assert!(stdout.contains("2: <expression> at"), "{stdout}");
    assert!(stdout.contains("local = 7"), "{stdout}");
    assert!(
        stdout.contains("inspect with #frames or #frame 0"),
        "{stdout}"
    );
    assert!(stdout.contains("restarts: #retry, #abort"), "{stdout}");
    assert!(stdout.contains("pending condition discarded"), "{stdout}");
    assert!(stdout.contains("no pending condition"), "{stdout}");
    assert!(!stdout.contains("__gin"), "{stdout}");
}

#[test]
fn standalone_terminal_evaluates_integer_addition() {
    let package = TempPackage::new("interpreter_terminal_standalone");
    package.write_flask("interpreter_terminal_standalone");
    package.write("numbers/defs.gin", NUMBERS);
    let output = run(
        &package,
        "use 'numbers' Signed64, signed64_add_bits, signed64_add\n2 + 2\n#q\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stderr:\n{stderr}");
    assert!(stdout.contains("4\n"), "stdout:\n{stdout}");
}

#[test]
fn terminal_recovers_after_diagnostic_and_executes_again() {
    let package = TempPackage::new("interpreter_terminal");
    package.write_flask("interpreter_terminal");
    package.write("numbers/defs.gin", NUMBERS);
    let output = run(
        &package,
        "use 'numbers' Signed64, signed64_add_bits, signed64_add\n40 + 2\nmissing + 2\n40 + 2\n#q\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert_eq!(
        stdout.matches("42\n").count(),
        2,
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("type-unknown-symbol"), "stderr:\n{stderr}");
}

#[test]
fn terminal_exposes_queries_scope_and_abort() {
    let package = TempPackage::new("interpreter_terminal_queries");
    package.write_flask("interpreter_terminal_queries");
    package.write("numbers/defs.gin", NUMBERS);
    let output = run(
        &package,
        "use 'numbers' Signed64, signed64_add_bits, signed64_add\n#type 2 + 2\n#scope\nmissing + 2\nmissing Signed64 := 40\n#retry\nmissing2 + 2\n#abort\n#q\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "stderr:\n{stderr}");
    assert!(stdout.contains("integer"), "stdout:\n{stdout}");
    assert!(stdout.contains("signed64_add"), "stdout:\n{stdout}");
    assert!(stdout.contains("42"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("pending condition discarded"),
        "stdout:\n{stdout}"
    );
    assert!(stderr.contains("type-unknown-symbol"), "stderr:\n{stderr}");
}
