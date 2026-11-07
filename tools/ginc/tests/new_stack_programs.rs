//! The programs of `modules/popular_problems`, built by the new stack for the host and run
//! natively: each case feeds standard input and compares standard output and the exit status
//! with `cases.json` of the program's directory (or the cases written here where it has none).
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn programs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/popular_problems")
}

/// The directory of a program: one of the popular problems, or a program beside them in `modules`.
fn program_directory(name: &str) -> PathBuf {
    let popular = programs().join(name);
    if popular.exists() {
        popular
    } else {
        programs().join("..").join(name)
    }
}

struct Case {
    stdin: String,
    stdout: String,
    status: i32,
}

fn cases_of(name: &str) -> Vec<Case> {
    let path = program_directory(name).join("cases.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{path:?}"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|case| Case {
            stdin: case["stdin"].as_str().unwrap().to_owned(),
            stdout: case["stdout"].as_str().unwrap().to_owned(),
            status: case["status"].as_i64().unwrap_or(0) as i32,
        })
        .collect()
}

fn case(stdin: &str, stdout: &str, status: i32) -> Case {
    Case {
        stdin: stdin.to_owned(),
        stdout: stdout.to_owned(),
        status,
    }
}

/// Build the program against the core library for the host.
fn build_program(name: &str) -> (tempfile::TempDir, PathBuf) {
    let source = std::fs::read_to_string(program_directory(name).join("main.gin")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        format!("#settings\nmodule: \"{name}\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n"),
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), source).unwrap();
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    let diagnostics = String::from_utf8_lossy(&built.stderr);
    assert!(
        built.status.success(),
        "{name} does not build:\n{diagnostics}"
    );
    (directory, program)
}

/// Build the program and run every case.
fn check_program(name: &str, cases: Vec<Case>) {
    let (_directory, program) = build_program(name);
    for case in cases {
        let mut child = Command::new(&program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child.stdin.take().unwrap().write_all(case.stdin.as_bytes());
        let done = child.wait_with_output().unwrap();
        assert_eq!(
            (
                String::from_utf8_lossy(&done.stdout).into_owned(),
                done.status.code()
            ),
            (case.stdout.clone(), Some(case.status)),
            "{name} on {:?}",
            case.stdin
        );
    }
}

#[test]
fn hello_world_prints_ten_lines() {
    check_program(
        "hello_world",
        vec![case("", &"Hello, world!\n".repeat(10), 0)],
    );
}

#[test]
fn echo_copies_standard_input() {
    check_program(
        "echo",
        vec![
            case("", "", 0),
            case(
                "Gin input\nwith two lines\n",
                "Gin input\nwith two lines\n",
                0,
            ),
        ],
    );
}

#[test]
fn a_plus_b_adds_two_numbers_and_refuses_an_overflow() {
    check_program(
        "a_plus_b",
        vec![
            case("20 22\n", "42\n", 0),
            case("-3 -4\n", "-7\n", 0),
            case("9223372036854775807 -9223372036854775808\n", "-1\n", 0),
            // Negative controls: a sum outside 64 bits prints nothing and ends with status 1.
            case("9223372036854775807 1\n", "", 1),
            case("-1 -9223372036854775808\n", "", 1),
            // Controls: a number past 64 bits, no number, and a trailing non-digit are all handled.
            case("99999999999999999999 1\n", "", 1),
            case("", "", 1),
            case("1\n", "", 1),
            case("12x 30y\n", "42\n", 0),
        ],
    );
}

#[test]
fn fizzbuzz_counts_to_its_limit() {
    let mut cases = cases_of("fizzbuzz");
    // Controls: a count past what could be counted to is refused with status 1, and a negative one prints nothing.
    cases.push(case("2000000000000000\n", "", 1));
    cases.push(case("-4\n", "", 0));
    // No number, and a number past 64 bits: nothing printed, status 1.
    cases.push(case("", "", 1));
    cases.push(case("99999999999999999999\n", "", 1));
    cases.push(case("5x", "1\n2\nFizz\n4\nBuzz\n", 0));
    check_program("fizzbuzz", cases);
}

#[test]
fn looping_counter_counts_up_and_down() {
    let mut cases = cases_of("looping_counter");
    // Control: bounds too far apart to count through are refused with status 1.
    cases.push(case("0 9223372036854775807\n", "", 1));
    check_program("looping_counter", cases);
}

#[test]
fn disan_count_prints_the_even_numbers() {
    let mut cases = cases_of("disan_count");
    cases.push(case("-3\n", "", 0));
    cases.push(case("9223372036854775807\n", "", 1));
    check_program("disan_count", cases);
}

#[test]
fn factorials_stop_at_what_64_bits_hold() {
    let mut cases = cases_of("factorials");
    // 20! is the last that fits; 21! stops the run with status 1 after the twenty lines.
    let twenty = [
        "1",
        "2",
        "6",
        "24",
        "120",
        "720",
        "5040",
        "40320",
        "362880",
        "3628800",
        "39916800",
        "479001600",
        "6227020800",
        "87178291200",
        "1307674368000",
        "20922789888000",
        "355687428096000",
        "6402373705728000",
        "121645100408832000",
        "2432902008176640000",
    ];
    let lines: String = twenty.iter().map(|n| format!("{n}\n")).collect();
    cases.push(case("20\n", &lines, 0));
    cases.push(case("21\n", &lines, 1));
    cases.push(case("2000000000000000\n", "", 1));
    check_program("factorials", cases);
}

#[test]
fn fibonacci_stops_at_what_64_bits_hold() {
    let mut cases = cases_of("fibonacci");
    // The 92nd number is the last that fits; the 93rd would not, and its predecessor's sum stops the run.
    let mut numbers = vec![1u64, 1];
    while numbers.len() < 92 {
        let next = numbers[numbers.len() - 1] + numbers[numbers.len() - 2];
        numbers.push(next);
    }
    let lines: String = numbers.iter().map(|n| format!("{n}\n")).collect();
    cases.push(case("92\n", &lines, 0));
    cases.push(case("93\n", &lines, 1));
    check_program("fibonacci", cases);
}

#[test]
fn primes_are_found_by_trial_division() {
    let mut cases = cases_of("primes");
    cases.push(case("100\n", "2\n3\n5\n7\n11\n13\n17\n19\n23\n29\n31\n37\n41\n43\n47\n53\n59\n61\n67\n71\n73\n79\n83\n89\n97\n", 0));
    cases.push(case("0\n", "", 0));
    cases.push(case("1000001\n", "", 1));
    check_program("primes", cases);
}

#[test]
fn digital_root_repeats_the_digit_sum() {
    let mut cases = cases_of("digital_root");
    // The largest number: 9223372036854775807 has digit sum 88, then 16, then 7.
    cases.push(case("9223372036854775807\n", "7\n", 0));
    cases.push(case("-5\n", "", 1));
    check_program("digital_root", cases);
}

#[test]
fn rot13_rotates_letters_and_keeps_the_rest() {
    let mut cases = cases_of("rot13");
    cases.push(case("", "", 0));
    cases.push(case("AZaz@[`{ 09\n", "NMnm@[`{ 09\n", 0));
    // Every byte value goes through (255 stays 255).
    cases.push(case("h\u{e9}llo", "u\u{e9}yyb", 0));
    check_program("rot13", cases);
}

#[test]
fn four_function_calculator_refuses_what_64_bits_cannot_hold() {
    let mut cases = cases_of("four_function_calculator");
    cases.push(case("7 -1 4\n", "-7\n", 0));
    cases.push(case("-9223372036854775808 -1 4\n", "undefined\n", 0));
    cases.push(case(
        "-9223372036854775807 -1 4\n",
        "9223372036854775807\n",
        0,
    ));
    cases.push(case("-9 -2 4\n", "4\n", 0));
    cases.push(case("1 1 9\n", "invalid operation\n", 0));
    cases.push(case(
        "9223372036854775807 9223372036854775807 3\n",
        "undefined\n",
        0,
    ));
    check_program("four_function_calculator", cases);
}

/// A program that never ends: its first output is read and compared, then it is stopped.
fn check_endless(name: &str, stdin: &str, line: &str) {
    use std::io::Read;
    let (_directory, program) = build_program(name);
    let mut child = Command::new(&program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(stdin.as_bytes());
    let mut out = child.stdout.take().unwrap();
    let mut first = vec![0u8; 4096];
    out.read_exact(&mut first).unwrap();
    // Still running after 4 KiB of output: it does not end by itself.
    assert!(child.try_wait().unwrap().is_none(), "{name} ended");
    child.kill().unwrap();
    let _ = child.wait();
    let text = String::from_utf8_lossy(&first).into_owned();
    let body = text.rsplit_once('\n').map_or("", |(whole, _)| whole);
    assert!(
        body.split('\n').all(|each| each == line),
        "{name}: {body:?}"
    );
}

#[test]
fn infinite_loop_never_ends() {
    check_endless("infinite_loop", "", "loop");
}

#[test]
fn truth_machine_prints_zero_once_and_one_for_ever() {
    check_program(
        "truth_machine",
        vec![
            case("0\n", "0\n", 0),
            case("7\n", "7\n", 0),
            case("-3\n", "-3\n", 0),
        ],
    );
    check_endless("truth_machine", "1\n", "1");
}

#[test]
fn deadfish_interprets_its_four_commands() {
    let mut cases = cases_of("deadfish_interpreter");
    cases.push(case("", "", 0));
    cases.push(case("xyz io\nio", "1\n2\n", 0));
    // 2 squared six times is 2^64, past 64 bits: the run stops with status 1 and prints nothing more.
    let squares = "ii".to_owned() + &"s".repeat(6) + "o";
    cases.push(case(
        &("ii".to_owned() + &"s".repeat(5) + "o"),
        "4294967296\n",
        0,
    ));
    cases.push(case(&squares, "", 1));
    cases.push(case("iiiiio", "5\n", 0));
    check_program("deadfish_interpreter", cases);
}

#[test]
fn mixed_write_runtime_prints_text_and_a_number() {
    check_program("mixed_write_runtime", vec![case("", "mixed\n7\n", 0)]);
}

#[test]
fn interpreter_demo_ends_with_the_low_byte_of_its_sum() {
    // factorial(6) + fibonacci(10) = 720 + 55 = 775, whose low byte is 7.
    check_program("interpreter_demo", vec![case("", "", 7)]);
}

#[test]
fn machine_add_adds_with_one_instruction() {
    // The sum is one hand-assembled `add` (no overflow check: the machine wraps), so the controls are
    // the readable cases and an unreadable input, which exits 1.
    check_program(
        "machine_add",
        vec![
            case("20 22\n", "42\n", 0),
            case("-5 3\n", "-2\n", 0),
            case("9223372036854775807 1\n", "-9223372036854775808\n", 0),
            case("1\n", "", 1),
        ],
    );
}

/// Build a program from its own directory, with the dependencies its `mod.flask` names.
fn build_package(name: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(program_directory(name))
        .output()
        .unwrap();
    let diagnostics = String::from_utf8_lossy(&built.stderr);
    assert!(
        built.status.success(),
        "{name} does not build:\n{diagnostics}"
    );
    (directory, program)
}

#[test]
fn log_level_test_prints_each_level_as_its_name() {
    let (_directory, program) = build_package("log_level_test");
    let cases = cases_of("log_level_test");
    assert_eq!(cases.len(), 1);
    let done = Command::new(&program).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&done.stdout), cases[0].stdout);
    assert_eq!(done.status.code(), Some(0));
    // Negative control: `Info` is never logged, so its name never appears.
    assert!(!String::from_utf8_lossy(&done.stdout).contains("info"));
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_demo_moves_the_pen_on_a_pty_and_refuses_a_pipe() {
    use std::time::Duration;
    let (_directory, program) = build_package("terminal_demo");
    // On a pseudo-terminal (`script` makes one): right, down, then `q`. Each key is sent after
    // the program has had time to enter raw mode, which discards what came before.
    let mut child = Command::new("/usr/bin/script")
        .args(["-q", "/dev/null"])
        .arg(&program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for key in [&b"\x1b[C"[..], b"\x1b[B", b"q"] {
        std::thread::sleep(Duration::from_millis(400));
        input.write_all(key).unwrap();
        input.flush().unwrap();
    }
    std::thread::sleep(Duration::from_millis(400));
    drop(input);
    let done = child.wait_with_output().unwrap();
    let screen = String::from_utf8_lossy(&done.stdout).into_owned();
    assert_eq!(done.status.code(), Some(0), "{screen:?}");
    assert!(
        screen.contains("Gin terminal: arrows move, q or Ctrl-C exits"),
        "{screen:?}"
    );
    // The pen starts at column 10, row 5 (1-based 6;11), moves right, then down.
    for place in ["\x1b[6;11H", "\x1b[6;12H", "\x1b[7;12H"] {
        assert!(screen.contains(place), "{place:?} in {screen:?}");
    }
    assert!(screen.ends_with("\x1b[?25h\x1b[?1049l"), "{screen:?}");
    // Negative control: a pipe is no terminal, so raw mode cannot be entered and it exits 1.
    let mut child = Command::new(&program)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"q").unwrap();
    let done = child.wait_with_output().unwrap();
    assert_eq!(done.status.code(), Some(1));
}
