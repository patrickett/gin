mod common;

use common::{Options, compile_and_run_with_options, compile_exe, require_success};
use ginc::cli::Profile;

#[test]
fn library_domain_bounds_execute_in_debug_and_release() {
    let source = r#"
Interval(lo to hi) is lo to hi
Interval(lo to hi) has
    lower: lo
    upper: hi
Word is in Interval(-128 to 127)
Word has
    finish(self):
        return
    maximum() Self: return Interval(domain of Self).upper as Self
#entry
main: Word.maximum()
"#;
    for profile in [Profile::Debug, Profile::Release] {
        compile_and_run_with_options(
            source,
            Options {
                profile,
                test_name: "library_domain_bounds".into(),
            },
        )
        .assert_compiled()
        .assert_exit_code(127);
    }
}

#[test]
fn imported_library_range_defines_an_integer_domain() {
    use std::{fs, process::Command};

    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("mod.flask"),
        r#"#settings
module: "domain_sample"
version: 0.0.0

#dependencies
"#,
    )
    .unwrap();
    fs::create_dir(directory.path().join("interval")).unwrap();
    fs::write(
        directory.path().join("interval/interval.gin"),
        "Interval(lo to hi) is lo to hi\nInterval(lo to hi) has\n    lower: lo\n    upper: hi\n",
    )
    .unwrap();
    fs::write(directory.path().join("main.gin"), "use domain_sample.interval Interval\nWord is in Interval(-128 to 127)\nWord has\n    finish(self):\n        return\n    maximum() Self: return Interval(domain of Self).upper as Self\n#entry\nmain: Word.maximum()\n").unwrap();
    let executable = directory.path().join("domain_sample");
    let result = compile_exe(directory.path().into(), &executable.clone(), Profile::Debug);
    require_success(&result, Profile::Debug);
    assert!(!result.has_typecheck_diagnostics(), "{result:?}");
    assert_eq!(Command::new(executable).status().unwrap().code(), Some(127));
}

#[test]
fn imported_domain_membership_checks_boundaries_before_narrowing() {
    use std::{fs, process::Command};

    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("mod.flask"),
        r#"#settings
module: "membership_sample"
version: 0.0.0

#dependencies
"#,
    )
    .unwrap();
    fs::create_dir(directory.path().join("numbers")).unwrap();
    fs::write(
        directory.path().join("numbers/numbers.gin"),
        "Word is in -32768 to 32767\nWord has\n    finish(self):\n        return\nOctet is in 0 to 255\nOctet has\n    finish(self):\n        return\n",
    )
    .unwrap();
    fs::write(directory.path().join("main.gin"), r#"
use membership_sample.numbers Word, Octet
convert(value Word) Octet: return when value is in Octet then value as Octet else 0
#entry
main:
    negative is in Octet: convert(-1)
    zero is in Octet: convert(0)
    positive is in Octet: convert(42)
    maximum is in Octet: convert(255)
    overflow is in Octet: convert(256)
    return when negative = 0 and zero = 0 and positive = 42 and maximum = 255 and overflow = 0 then 0 else 1
"#).unwrap();
    for profile in [Profile::Debug, Profile::Release] {
        let executable = directory.path().join("membership_sample");
        let result = compile_exe(directory.path().into(), &executable.clone(), profile);
        require_success(&result, profile);
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        assert_eq!(Command::new(executable).status().unwrap().code(), Some(0));
    }
}

#[test]
fn library_bit_width_can_widen_a_nominal_size_query() {
    let source = r#"
Interval(lo to hi) is lo to hi
Word is in Interval(-9223372036854775808 to 9223372036854775807)
Word has
    finish(self):
        return
    bit_width() Self:
        bytes is in Self: #size(Word)
        return (bytes * 8) as Self
#entry
main: Word.bit_width
"#;
    for profile in [Profile::Debug, Profile::Release] {
        compile_and_run_with_options(
            source,
            Options {
                profile,
                test_name: "library_bit_width".into(),
            },
        )
        .assert_compiled()
        .assert_exit_code(64);
    }
}
