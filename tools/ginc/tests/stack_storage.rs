mod common;
use common::compile_exe;
use ginc::cli::Profile;
use std::process::Command;
use test_fixtures::TempPackage;

fn check(source: &str, name: &str) {
    let package = TempPackage::new(name);
    package.write_flask(name);
    let input = package.write("main.gin", source);
    for profile in [Profile::Debug, Profile::Release] {
        let output = package.join(format!("program-{profile:?}"));
        let result = compile_exe(input.clone(), &output.clone(), profile);
        assert!(result.is_success(), "{name} {profile:?}: {result:?}");
        assert!(
            !result.has_typecheck_diagnostics(),
            "{name} {profile:?}: {result:?}"
        );
        let status = Command::new(output).status().unwrap();
        assert_eq!(status.code(), Some(0), "{name} {profile:?}: {status}");
    }
}

#[test]
fn loop_with_early_return_does_not_accumulate_stack_storage() {
    check(
        r#"Word is in 0 to 9223372036854775807
Word has
    finish(self):
        return

exercise(count is in Word) Word:
    for index in 0 to count
        value: index
        when value = 1999999 then
            return 0
    return 1

#entry
main: exercise(2000000)
"#,
        "loop_stack_storage",
    );
}

#[test]
fn nested_loop_locals_are_reinitialized_and_remain_addressable() {
    check(
        r#"Word is in 0 to 9223372036854775807
Word has
    finish(self):
        return

increment(value mut Word):
    bits is in Word: value
    value:: ((bits + 1) & 255) as Word
    return

exercise(count is in Word) Word:
    for outer in 0 to count
        for inner in 0 to count
            value Word: 7
            increment(value)
            bits is in Word: value
            when not (bits = 8) then
                return 1
    return 0

#entry
main: exercise(1500)
"#,
        "nested_loop_stack_storage",
    );
}
