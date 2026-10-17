use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn template_strings_are_rejected_without_emitting_an_artifact() {
    let package = TempPackage::new("deferred_template_todo");
    let input = package.write("main.gin", "#entry\nmain: \"value \\(42)\"\n");
    let executable = package.join("program");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg(input)
        .args(["--target", &host_target_triple(), "--no-cache", "-o"])
        .arg(&executable)
        .env("GIN_HOME", package.join("home"))
        .output()
        .unwrap();
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostics.contains("parse-template-strings-not-supported"),
        "{diagnostics}"
    );
    assert!(!output.status.success(), "{diagnostics}");
    assert!(!executable.exists());
}

#[test]
fn template_methods_execute_as_ordinary_user_definitions() {
    let package = TempPackage::new("ordinary_template_names");
    let input = package.write(
        "main.gin",
        &format!("{}Template has\n    value Word\n    finish(self):\n        return\nTemplate.render(self own) Word:\n    Template(value: result) := self\n    return result\nTemplate.release(self own) Word:\n    Template(value: result) := self\n    return result\nTemplateSink has\n    value Word\n    finish(self):\n        return\nTemplateSink.write(self own) Word:\n    TemplateSink(value: result) := self\n    return result\n#entry\nmain:\n    rendered: Template.render(Template(value: 42))\n    released: Template.release(Template(value: rendered))\n    return TemplateSink.write(TemplateSink(value: released))\n", test_fixtures::preludes::WORD_PRELUDE),
    );
    let executable = package.join("program");
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg(input)
        .args(["--target", &host_target_triple(), "--no-cache", "-o"])
        .arg(&executable)
        .env("GIN_HOME", package.join("home"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("flaw:"),
        "{output:?}"
    );
    let execution = Command::new(executable).output().unwrap();
    assert_eq!(execution.status.code(), Some(42), "{execution:?}");
}
