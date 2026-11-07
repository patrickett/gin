//! An `#instruction` is a block of the target's own assembly, with symbols placed by name.
//!
//! Real libc, called from Gin packages: `write` with its message as data in the block,
//! `printf` with variadic arguments on the stack, and `qsort` calling back into a Gin
//! function. Nothing here is known to the compiler: the ABI is the Gin text below.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]
use std::process::{Command, Output};

const TARGET: &str = include_str!("instruction_blocks/arm64.gin");
const LIBC: &str = include_str!("instruction_blocks/libc.gin");
const MODES: &str = include_str!("instruction_blocks/modes.gin");
const IMPORTS: &str = "use app.libc hello, numbers, sorted, put, both\nuse app.modes Mode\n";

/// The package every test builds: the target, the C library, a module of unions, and `main`.
fn lay_out(root: &std::path::Path, main: &str, libc: &str) {
    std::fs::write(root.join("main.gin"), format!("{IMPORTS}{main}")).unwrap();
    for (name, text) in [("arm64", TARGET), ("libc", libc), ("modes", MODES)] {
        let module = root.join(name);
        std::fs::create_dir(&module).unwrap();
        std::fs::write(module.join(format!("{name}.gin")), text).unwrap();
    }
    std::fs::write(
        root.join(flask::MANIFEST_NAME),
        "#settings\nmodule: 'app'\nversion: 0.0.0\n\n#dependencies\n",
    )
    .unwrap();
}

fn build(main: &str) -> (tempfile::TempDir, Output) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    lay_out(root, main, LIBC);
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(root.join("program"))
        .arg(root)
        .output()
        .unwrap();
    (directory, output)
}

/// Runs the built program, killing it if it does not finish: a block that is wrong can
/// loop, and a test must fail instead of waiting for it.
fn run(main: &str) -> Output {
    let (directory, built) = build(main);
    assert!(built.status.success(), "{built:?}");
    let child = Command::new(directory.path().join("program"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(child.wait_with_output()).unwrap());
    match receiver.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(output) => output.unwrap(),
        Err(_) => {
            let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
            panic!("the program did not finish");
        }
    }
}

#[test]
fn write_prints_a_message_that_is_data_in_the_block() {
    let output = run("#entry\nstart() in 0 to 255:\n    hello()\n    return 0\n");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hi\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn printf_takes_its_variadic_arguments_from_the_stack() {
    let output = run("#entry\nstart() in 0 to 255:\n    numbers(7, 35)\n    return 0\n");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "7 35\n");
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn the_stack_is_back_where_it_was_after_a_block() {
    let output = run(
        "#entry\nstart() in 0 to 255:\n    numbers(1, 2)\n    numbers(3, 4)\n    numbers(5, 6)\n    return 9\n",
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1 2\n3 4\n5 6\n");
    assert_eq!(output.status.code(), Some(9));
}

/// `qsort` reaches a function Gin defines, by the name the library gave it.
#[test]
fn qsort_calls_back_into_a_gin_function() {
    let output = run("#entry\nstart() in 0 to 255:\n    sorted()\n    return 0\n");
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(!printed.is_empty(), "the callback was never called");
    assert!(printed.bytes().all(|byte| byte == b'.'), "{printed:?}");
    assert_eq!(output.status.code(), Some(0));
}

/// A block and a single instruction in one program, both through stdio so that they share
/// libc's buffer and keep the order they were called in.
#[test]
fn blocks_and_single_instructions_mix_in_one_program() {
    let output = run(
        "#entry\nstart() in 0 to 255:\n    put(65)\n    numbers(1, 2)\n    put(66)\n    return 0\n",
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "A1 2\nB");
}

/// `write` goes straight to the descriptor while stdio waits to be flushed at exit, so a
/// message written between two buffered ones arrives first: that is libc, not the compiler.
#[test]
fn write_is_not_buffered_and_stdio_is() {
    let output =
        run("#entry\nstart() in 0 to 255:\n    put(65)\n    hello()\n    put(66)\n    return 0\n");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Hi\nAB");
}

/// A misspelled callback is not a type error, but it cannot link: the build fails.
#[test]
fn a_callback_that_names_no_function_does_not_link() {
    let libc = LIBC.replace("'app.libc.tick'", "'app.libc.tock'");
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    lay_out(
        root,
        "#entry\nstart() in 0 to 255:\n    sorted()\n    return 0\n",
        &libc,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(root.join("program"))
        .arg(root)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let reason = String::from_utf8_lossy(&output.stderr);
    assert!(reason.contains("linking failed"), "{reason}");
    assert!(reason.contains("app.libc.tock"), "{reason}");
    assert!(!root.join("program").exists());
}

/// The address of another module's function and a call to it in the same module name the
/// same declaration, so the module is accepted and the program runs.
#[test]
fn a_function_of_another_module_may_be_called_and_handed_to_c() {
    let (directory, built) =
        build("#entry\nstart() in 0 to 255:\n    both(Mode.Loud)\n    return 0\n");
    assert!(built.status.success(), "{built:?}");
    let status = Command::new(directory.path().join("program"))
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(0));
}
