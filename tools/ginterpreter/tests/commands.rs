//! Session commands start with `$`; `#` is the language's. The two namespaces are disjoint by
//! construction, and these tests check it against the real lexer and parser: nothing the
//! language accepts starts with `$`, and nothing starting with `#` is ever a command.
use std::io::Write;
use std::process::{Command, Stdio};

use ginterpreter::commands::{REPL_COMMANDS, all_names, repl_command, suggestion};

fn repl(input: &str) -> (String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ginterpreter")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    (String::from_utf8(output.stdout).unwrap(), String::from_utf8(output.stderr).unwrap())
}

/// Every place a submission could put a command name: it starts a file, a line, a member, an
/// expression, an argument.
fn positions(name: &str) -> Vec<String> {
    vec![
        format!("${name}\n"),
        format!("${name} x\n"),
        format!("${name}(x)\n"),
        format!("f(x in 0 to 9) in 0 to 9: return ${name}\n"),
        format!("f(x in 0 to 9) in 0 to 9: return ${name}(x)\n"),
        format!("T has\n    ${name}\n    f(self) in 0 to 9: return 1\n"),
        format!("T has\n    ${name} x in 0 to 9\n"),
        format!("${name}: 1\n"),
    ]
}

#[test]
fn the_language_accepts_nothing_that_starts_with_a_dollar() {
    let names = all_names();
    assert!(names.len() >= 15, "{names:?}");
    for name in names {
        for source in positions(name) {
            let diagnostics = parser::parse_strict(&source).expect_err(&format!("the language accepted {source:?}"));
            assert!(
                diagnostics.iter().any(|d| d.message.contains("unexpected character")),
                "{source:?}: {:?}",
                diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn a_dollar_that_is_not_at_the_start_stays_whatever_the_language_says() {
    // Controls: the two places the language does take a `$` (a comment and the text of an
    // instruction), so the test above is about the start of a submission, not about `$` itself.
    assert!(parser::parse_strict("-- $inspect is only a comment\nf() in 0 to 9: return 1\n").is_ok());
    assert!(parser::parse_strict("f() in 0 to 9: return 1 -- $inspect\n").is_ok());
    assert!(parser::parse_strict("#instruction('bl $target')\ntrap(a Type) a\n").is_ok());
    // A `$` in code is the language's error, wherever it is.
    assert!(parser::parse_strict("x: 1 $ 2\n").is_err());
    // And never a command: only a `$` at the very start of a submission is read as one.
    assert_eq!(repl_command("five() $inspect"), None);
    assert_eq!(repl_command("f() in 0 to 9: return 1 -- $reset"), None);
    assert_eq!(repl_command("x: $quit"), None);
    // The session accepts a declaration that mentions a command in a comment, and reports the
    // language's error for one in code, without running anything.
    let (out, err) = repl("five() in 0 to 9: return 5 -- $reset\n\nfive()\n\nfive() $reset\n\nfive()\n");
    assert!(out.contains("ok: added fn five/0"), "{out}");
    assert_eq!(out.matches("5 : 5 to 5").count(), 2, "the definition survived: {out}");
    assert!(err.contains("unexpected character"), "{err}");
    assert!(!out.contains("session cleared"), "{out}");
}

#[test]
fn a_hash_line_is_never_a_command() {
    for name in all_names() {
        for line in [format!("#{name}"), format!("#{name} x"), format!("  #{name}"), format!("#{name}(x)")] {
            assert_eq!(repl_command(&line), None, "{line}");
        }
    }
    // Including `#inspect`, `#quit`, `#reset`: the language reports an unknown directive, and the
    // session goes on.
    let (out, err) = repl("five() in 0 to 9: return 5\n#inspect five\n\n#reset\n\n#quit\n\nfive()\n");
    assert!(err.contains("inspect") && err.contains("reset") && err.contains("quit"), "{err}");
    assert!(!out.contains("\"found\""), "no inspection happened: {out}");
    assert!(!out.contains("session cleared"), "{out}");
    assert!(out.contains("5 : 5 to 5"), "the session was not ended or cleared: {out}");
}

#[test]
fn a_command_is_its_name_after_a_dollar_and_nothing_longer() {
    assert_eq!(repl_command("$inspect Point"), Some(("inspect", "Point")));
    assert_eq!(repl_command("  $reset"), Some(("reset", "")));
    assert_eq!(repl_command("$restore a.json --verify"), Some(("restore", "a.json --verify")));
    assert_eq!(repl_command("$inspector Point"), None);
    assert_eq!(repl_command("$nonsense"), None);
    assert_eq!(repl_command(":inspect Point"), None);
    for name in REPL_COMMANDS {
        assert!(repl_command(&format!("${name}")).is_some(), "{name}");
    }
}

#[test]
fn an_unknown_operation_suggests_the_dollar_spelling_only_when_the_word_matches() {
    for spelled in ["inspect", ":inspect", "#inspect", "$inspect"] {
        assert_eq!(suggestion(spelled).as_deref(), Some("$inspect"), "{spelled}");
    }
    assert_eq!(suggestion("nonsense"), None);
    assert_eq!(suggestion("#nonsense"), None);
}
