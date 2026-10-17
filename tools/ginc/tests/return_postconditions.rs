mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

const MODEL: &str = r#"Numeric has
    as_num(self ref) is in Self: return (number is in Self: self)
Count is in 0 to 255
Count has Numeric
    release(self):
        return
replace(previous Count, next Count) Count: return next
Region has
    capacity Count
    initialized Count and <= capacity
    initialized_count(self ref) is in Count: return self.initialized.as_num
    capacity_count(self ref) is in Count: return self.capacity.as_num
    empty(self ref and self.initialized_count = 0) Count: return 0
    pop(self mut and self.initialized_count > 0) Count:
        count: self.initialized.as_num
        self.initialized :: replace(self.initialized, (count - 1) as Count)
        return 0
    release(self):
        return
clear(region mut Region):
    while region.initialized_count > 0 do
        value: region.pop
        value.release
    return thus region.initialized_count = 0
"#;
fn run(source: &str, profile: Profile, expected: i32) {
    let package = TempPackage::new("return_postconditions");
    let input = package.write("main.gin", source);
    let executable = package.join("program");
    let output = compile_and_run_exe(input, &executable, profile, "", "run verified cleanup");
    assert_eq!(output.status.code(), Some(expected));
}

#[test]
fn verified_cleanup_runs_for_empty_and_nonempty_records() {
    for profile in [Profile::Debug, Profile::Release] {
        for initialized in [0, 1, 4] {
            let source = format!(
                r#"{MODEL}#entry
main:
    region: Region(capacity: 4, initialized: {initialized})
    clear(region)
    zero: region.empty
    capacity: region.capacity_count
    zero.release
    region.release
    return capacity
"#
            );
            run(&source, profile, 4);
        }
    }
}

#[test]
fn returning_a_mutated_record_preserves_its_current_fields() {
    let source = format!(
        r#"{MODEL}clear_local(region mut Region) Region:
    local: Region(capacity: 4, initialized: 1)
    clear(local)
    return local thus local.initialized_count = 0
#entry
main:
    original: Region(capacity: 4, initialized: 4)
    result: clear_local(original)
    count: result.initialized_count
    result.release
    original.release
    return count
"#
    );
    for profile in [Profile::Debug, Profile::Release] {
        run(&source, profile, 0);
    }
}
