mod common;

use common::{compile_and_run_exe, compile_exe};
use ginc::cli::Profile;
use ginc::compile::{CompileFailure, CompileResult};
use test_fixtures::TempPackage;

#[test]
fn temporary_projection_leaks_stop_compilation() {
    for body in [
        r#"bad(first Leaf, second Leaf):
    Pair(first: first, second: second).first.release
    return
"#,
        r#"bad(first Leaf, second Leaf) Leaf: return make(first, second).first
"#,
        r#"bad(first Leaf, second Leaf):
    [first, second].[0].release
    return
"#,
        r#"bad(first Leaf, second Leaf, flag ref Word):
    when flag is 0 then
        make(first, second).first.release
        return
    finish(make(first, second))
    return
"#,
        r#"Maybe is Present(Leaf) or Absent
wrap(value own Leaf) Maybe: return Present(value)
bad(value Leaf):
    when wrap(value) is Absent then idle
    return
idle: return
"#,
    ] {
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("temporary_projection");
            package.write_flask("temporary_projection");
            package.write(
                "resources/resources.gin",
                r#"Word is in 0 to 255
Word.finish(self): return
Leaf has
    state Word
    release(self): return
"#,
            );
            let input = package.write(
                "main.gin",
                &format!(
                    r#"use temporary_projection.resources Word, Leaf
Pair has
    first Leaf
    second Leaf
make(first own Leaf, second own Leaf) Pair: return Pair(first: first, second: second)
finish(pair Pair):
    pair.first.release
    pair.second.release
    return
{body}#entry
main: 0
"#
                ),
            );
            let executable = package.join("rejected");
            let result = compile_exe(input, &executable, profile);
            assert!(
                matches!(
                    result,
                    CompileResult::Failed(CompileFailure::TypecheckFailed)
                ),
                "{profile:?}, {body}: {result:?}"
            );
            assert!(!executable.exists(), "{profile:?}, {body}");
        }
    }
}

#[test]
fn temporary_projections_transfer_resources_without_duplicate_cleanup() {
    for flag in [0, 1] {
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("temporary_transfers");
            package.write_flask("temporary_transfers");
            package.write(
                "resources/resources.gin",
                r#"Word is in 0 to 255
Word has
    finish(self): return
    replace(previous Self, next Self) Self: return next
Leaf has
    count @Word
    release(self):
        pointer: self.count
        previous is in Word: pointer.[0]
        when previous < 255 then
            pointer.[0]:: Word.replace(pointer.[0], (previous + 1) as Word)
        return
"#,
            );
            let input = package.write(
                "main.gin",
                &format!(
                    r#"use temporary_transfers.resources Word, Leaf
Single has value Leaf
Mixed has
    value Leaf
    state Word
Nested has value Single
ArrayHolder has items [Leaf; 1]
make(value own Leaf) Single: return Single(value: value)
finish(value Leaf): return value.release
new(count ref @Word) Leaf: return Leaf(count: deref count)
exercise(count ref @Word, flag ref Word):
    Single(value: new(count)).value.release
    make(new(count)).value.release
    [new(count)].[0].release
    Mixed(value: new(count), state: 0).value.release
    finish(Nested(value: Single(value: new(count))).value.value)
    when flag is 0 then
        Single(value: new(count)).value.release
    else
        make(new(count)).value.release
    ArrayHolder(items: [new(count)]).items.[0].release
    for index in 0 to 2
        Single(value: new(count)).value.release
    return
#entry
main:
    count Word: 0
    flag Word: {flag}
    pointer: (@count) as @Word
    exercise(pointer, flag)
    return count
"#
                ),
            );
            let executable = package.join("program");
            let output =
                compile_and_run_exe(input, &executable, profile, "", "run temporary transfers");
            assert_eq!(output.status.code(), Some(10), "{profile:?}, flag {flag}");
        }
    }
}
