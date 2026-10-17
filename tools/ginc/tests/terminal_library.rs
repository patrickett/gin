#![cfg(unix)]

mod common;
use common::{compile_exe, require_success};

use std::path::{Path, PathBuf};
use std::process::Command;

use ginc::cli::Profile;
use test_fixtures::{TempPackage, host_target_triple};

fn modules() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../modules")
        .canonicalize()
        .unwrap()
}

fn prepare_package(package: &TempPackage) {
    for name in ["core", "terminal"] {
        package.copy_tree(&modules().join(name), name);
    }
    package.write(
        "mod.flask",
        &format!("#settings\nmodule: 'terminal_test'\nversion: 0.1.0\ntarget: '{}'\n#dependencies\ncore: './core'\nterminal: './terminal'\n", host_target_triple()),
    );
}

fn compile(package: &TempPackage, source: &str, profile: Profile) -> PathBuf {
    compile_with_write_result(package, source, profile, None)
}

fn compile_with_write_result(
    package: &TempPackage,
    source: &str,
    profile: Profile,
    write_result: Option<&str>,
) -> PathBuf {
    prepare_package(package);
    if let Some(write_result) = write_result {
        let transfer = std::fs::read_to_string(modules().join("terminal/io/io.gin")).unwrap();
        let write_start = transfer.rfind("\nwrite(").unwrap();
        package.write(
            "terminal/io/io.gin",
            &format!(
                concat!(
                    "{}\n",
                    "write(fd SystemResult, pointer BytePointer, size MemorySize) TransferResult:\n",
                    "    return TransferResult.{}\n",
                ),
                &transfer[..write_start],
                write_result,
            ),
        );
    }
    let input = package.write("main.gin", source);
    let output = package.join(format!("program-{profile:?}"));
    let result = compile_exe(input, &output.clone(), profile);
    require_success(&result, profile);
    assert!(
        !result.has_typecheck_diagnostics(),
        "{profile:?}: {result:?}"
    );
    output
}

#[test]
fn sessions_require_close_directly_and_inside_open_results() {
    let package = TempPackage::new("terminal_session_obligations");
    prepare_package(&package);
    for (body, pending) in [
        ("forget(session Session): return\n", "session"),
        ("forget(result OpenResult): return\n", "result"),
        (
            "forget(result OpenResult): return when result is Opened(_) then 0 else 1\n",
            "result",
        ),
    ] {
        let input = package.write(
            "main.gin",
            &format!("use terminal Session, OpenResult\n{body}#entry\nmain: 0\n"),
        );
        let output = package.join("rejected.o");
        let result = Command::new(env!("CARGO_BIN_EXE_ginc"))
            .arg(input)
            .args(["--emit", "obj", "--no-cache", "--output"])
            .arg(&output)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(!result.status.success(), "{body}: {stderr}");
        assert!(
            stderr.contains(&format!("[type-terminal-value-not-consumed] `{pending}`")),
            "{body}: {stderr}"
        );
        assert!(!output.exists(), "{body}");
    }
}

#[test]
fn output_is_buffered_bounded_and_preserved_on_write_failure() {
    let source = r#"use terminal.output Output
use core.alloc.system SystemResult
use core.slice Length

#entry
main:
    writer: Output.new(1)
    full is in SystemResult: 0
    for block in 0 to 255
        full :: writer.queue('0123456789abcdef').to_num
    rejected: when writer.queue('overflow') is OutputFailed(BufferFull) then 0 else 1
    rejected_color: when writer.foreground(3) is OutputFailed(BufferFull) then 0 else 1
    rejected_move: when writer.move_to(0, 0) is OutputFailed(BufferFull) then 0 else 1
    pending is in Length: writer.pending
    writer.discard
    writer.enter_alternate_screen
    writer.hide_cursor
    writer.clear
    writer.move_to(0, 0)
    writer.foreground(6)
    writer.queue('Gin terminal: arrows move, q or Ctrl-C exits')
    writer.move_to(65535, 65535)
    writer.reset_style
    writer.show_cursor
    writer.leave_alternate_screen
    marker: Output.new(1)
    marker.queue('queued\n')
    marker.flush
    flushed: when writer.flush is OutputSucceeded then 0 else 1
    empty is in Length: writer.pending
    failed: Output.new(-1)
    failed.queue('retry')
    error: when failed.flush is
        OutputFailed(error) then error.to_num
        else 0
    retained is in Length: failed.pending
    failed.discard
    return when full = 0 and rejected = 0
            and rejected_color = 0 and rejected_move = 0
            and pending = 4096 and flushed = 0 and empty = 0
            and error = -9 and retained = 5 then 0 else 1
"#;
    let expected = concat!(
        "queued\n\x1b[?1049h\x1b[?25l\x1b[2J\x1b[1;1H\x1b[36m",
        "Gin terminal: arrows move, q or Ctrl-C exits",
        "\x1b[65536;65536H\x1b[0m\x1b[?25h\x1b[?1049l",
    )
    .as_bytes();
    let package = TempPackage::new("terminal_output");
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, source, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert_eq!(output.stdout, expected, "{profile:?}");
    }
}

#[test]
fn stalled_and_interrupted_writes_preserve_buffered_output() {
    for (write_result, error_arm) in [
        ("Transferred(0)", "WriteZero then 0"),
        (
            "Transferred(-1)",
            "WriteFailed(code) then when code = -5 then 0 else 1",
        ),
        (
            "Transferred(6)",
            "WriteFailed(code) then when code = -5 then 0 else 1",
        ),
        ("Interrupted", "RetryLimitReached then 0"),
    ] {
        let source = format!(
            r#"use terminal.output Output
use core.slice Length

#entry
main:
    writer: Output.new(1)
    writer.queue('retry')
    result: writer.flush
    expected: when result is
        OutputFailed(error) then when error is
            {error_arm}
            else 1
        else 1
    remaining is in Length: writer.pending
    writer.discard
    return when expected = 0 and remaining = 5 then 0 else 1
"#,
        );
        let package = TempPackage::new("terminal_stalled_output");
        for profile in [Profile::Debug, Profile::Release] {
            let executable =
                compile_with_write_result(&package, &source, profile, Some(write_result));
            let output = Command::new(executable).output().unwrap();
            assert!(output.status.success(), "{profile:?}: {output:?}");
            assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
        }
    }
}

#[test]
fn decoder_preserves_fragmented_arrows_and_distinguishes_escape_and_interrupt() {
    let source = r#"use terminal.event Decoder

#entry
main:
    decoder: Decoder.new
    first: when decoder.feed(27) is Pending then 0 else 1
    second: when decoder.feed(91) is Pending then 0 else 1
    up: when decoder.feed(65) is Up then 0 else 1
    decoder.feed(27)
    decoder.feed(79)
    down: when decoder.feed(66) is Down then 0 else 1
    decoder.feed(27)
    decoder.feed(91)
    left: when decoder.feed(68) is Left then 0 else 1
    decoder.feed(27)
    decoder.feed(91)
    right: when decoder.feed(67) is Right then 0 else 1
    decoder.feed(27)
    escape: when decoder.timeout is Escape then 0 else 1
    decoder.feed(27)
    decoder.feed(91)
    incomplete: when decoder.timeout is Pending then 0 else 1
    interrupt: when decoder.feed(3) is Interrupt then 0 else 1
    decoder.feed(27)
    interrupted_escape: when decoder.feed(3) is Interrupt then 0 else 1
    decoder.feed(27)
    decoder.feed(91)
    interrupted_arrow: when decoder.feed(3) is Interrupt then 0 else 1
    ordinary: when decoder.feed(113) is ByteKey(113) then 0 else 1
    return first + second + up + down + left + right + escape + incomplete + interrupt
           + interrupted_escape + interrupted_arrow + ordinary
"#;
    let package = TempPackage::new("terminal_decoder");
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, source, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
    }
}

fn pty_check(executable: &Path, mode: &str) {
    let result = Command::new("python3")
        .args(["-c", include_str!("support/terminal_pty.py")])
        .arg(executable)
        .arg(mode)
        .output()
        .expect("python3 is required for terminal integration tests");
    assert!(result.status.success(), "{result:?}");
}

#[test]
fn settings_copies_are_infallible_and_independent() {
    let source = r#"use terminal.platform Settings

#entry
main:
    original: Settings.new
    captured: original.capture(0).to_num
    raw: original.copy
    saved: raw.copy
    activated: raw.activate_raw(0).to_num
    restored: saved.restore(0).to_num
    return when captured = 0 and activated = 0 and restored = 0 then 0 else 1
"#;
    let package = TempPackage::new("terminal_settings_copy");
    for profile in [Profile::Debug, Profile::Release] {
        pty_check(&compile(&package, source, profile), "settings_copy");
    }
}

#[test]
fn demo_moves_with_arrows_and_restores_terminal_on_quit_and_interrupt() {
    let source = std::fs::read_to_string(modules().join("terminal_demo/main.gin")).unwrap();
    let package = TempPackage::new("terminal_demo");
    for profile in [Profile::Debug, Profile::Release] {
        let executable = compile(&package, &source, profile);
        pty_check(&executable, "demo");
        let redirected = Command::new(&executable).output().unwrap();
        assert_eq!(redirected.status.code(), Some(1));
        assert!(redirected.stdout.is_empty());
    }
}

#[test]
fn named_descriptors_map_at_the_system_boundary() {
    let source = r#"use core.io.descriptor Descriptor

#entry
main:
    input: Descriptor.StandardInput.to_num.to_num
    output: Descriptor.StandardOutput.to_num.to_num
    error: Descriptor.StandardError.to_num.to_num
    raw: Descriptor.Raw(7).to_num.to_num
    return when input = 0 and output = 1 and error = 2 and raw = 7
           then 0 else 1
"#;
    let package = TempPackage::new("terminal_descriptors");
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, source, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
    }
}

#[test]
fn invalid_named_descriptors_preserve_system_errors() {
    let source = r#"use core.io.descriptor Descriptor
use core.alloc.system SystemResult
use terminal Session

#entry
main: when Session.open(Descriptor.Raw(-1), Descriptor.StandardOutput) is
    Opened(s) then unexpected_open(s)
    OpenFailed(code) then expected_error(code)

unexpected_open(s Session):
    s.close
    return 1

expected_error(code SystemResult):
    value: code.to_num
    return when value = -9 then 0 else 1
"#;
    let package = TempPackage::new("terminal_invalid_descriptor");
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, source, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
    }
}

#[test]
fn failed_enter_still_restores_raw_mode() {
    let source = r#"use terminal EnterResult, Session
use core.io.descriptor Descriptor
use core.alloc.system SystemResult

#entry
main:
    return when Session.open(Descriptor.StandardInput, Descriptor.StandardOutput) is
        Opened(session) then exercise(session)
        OpenFailed(code) then 1

exercise(session Session):
    terminal: session
    inactive: terminal.read
    not_active: when inactive is
        ReadFailed(error) then when error.to_num = -22 then 0 else 1
        else 1
    for block in 0 to 255
        terminal.output.queue('0123456789abcdef')
    failed: terminal.enter
    expected: when failed is
        EnterOutputFailed(error) then when error.to_num = -28 then 0 else 1
        else 1
    repeated: terminal.enter
    already_active: when repeated is AlreadyActive then 0 else 1
    entered_code: EnterResult.Entered.to_num
    active_code: EnterResult.AlreadyActive.to_num
    failed_code: EnterResult.EnterFailed(-9).to_num
    closed: terminal.close
    output: closed.output.as_num
    settings is in SystemResult: closed.settings
    return when not_active = 0 and expected = 0 and already_active = 0
            and output = 0 and settings = 0
            and entered_code = 0 and active_code = -22 and failed_code = -9
           then 0 else 1
"#;
    let package = TempPackage::new("terminal_failed_enter");
    for profile in [Profile::Debug, Profile::Release] {
        pty_check(&compile(&package, source, profile), "failed_enter");
    }
}

#[test]
fn nominal_read_output_and_transfer_results_preserve_numeric_codes() {
    let source = r#"use terminal ReadError
use terminal.output OutputError, OutputResult
use terminal.io TransferResult

#entry
main:
    inactive: ReadError.NotActive.to_num
    read_failed: ReadError.ReadSystemError(-9).to_num
    full: OutputError.BufferFull.to_num
    zero: OutputError.WriteZero.to_num
    exhausted: OutputError.RetryLimitReached.to_num
    write_failed: OutputError.WriteFailed(-9).to_num
    succeeded: OutputResult.OutputSucceeded.to_num
    failed: OutputResult.OutputFailed(OutputError.BufferFull).to_num
    transferred: TransferResult.from_num(3).to_num
    interrupted: TransferResult.from_num(-4).to_num
    transfer_failed: TransferResult.from_num(-9).to_num
    return when inactive = -22 and read_failed = -9
            and full = -28 and zero = -5 and exhausted = -11 and write_failed = -9
            and succeeded = 0 and failed = -28
            and transferred = 3 and interrupted = -4 and transfer_failed = -9
           then 0 else 1
"#;
    let package = TempPackage::new("terminal_nominal_results");
    for profile in [Profile::Debug, Profile::Release] {
        let output = Command::new(compile(&package, source, profile))
            .output()
            .unwrap();
        assert!(output.status.success(), "{profile:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{profile:?}: {output:?}");
    }
}
