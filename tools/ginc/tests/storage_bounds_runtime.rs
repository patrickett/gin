mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use std::os::unix::process::ExitStatusExt;
use test_fixtures::TempPackage;

const TYPES: &str = r#"Byte is in 0 to 255
Index is in -128 to 127
WideIndex is in 0 to 18446744073709551616
Byte has
    finish(self):
        return
Index has
    finish(self):
        return
WideIndex has
    finish(self):
        return
#fixed_array(element, count)
Buffer(element, count BigInt) has
    finish(self):
        return
get(values ref Buffer(Byte, 4), index Index) Byte: return values.[index]
get_wide(values ref Buffer(Byte, 4), index WideIndex) Byte: return values.[index]
replace(previous Byte, next Byte) Byte: return next
set(values mut Buffer(Byte, 4), index Index):
    values.[index]:: replace(values.[index], 19)
    return
"#;

fn run(source: &str, trap: bool) {
    let package = TempPackage::new("storage_bounds");
    let input = package.write("main.gin", &format!("{TYPES}{source}"));
    for profile in [Profile::Debug, Profile::Release] {
        let executable = package.join(format!("program-{profile:?}"));
        let output = compile_and_run_exe(input.clone(), &executable, profile, "", "run bounds");
        if trap {
            assert!(
                matches!(output.status.signal(), Some(4 | 5)),
                "{profile:?}: expected a bounds trap, got {output:?}"
            );
        } else {
            assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
        }
    }
}

#[test]
fn dynamic_boundary_reads_and_writes_preserve_values() {
    run(
        r#"#entry
main:
    values Buffer(Byte, 4): [7, 11, 13, 17]
    first is in Byte: get(values, 0)
    last is in Byte: get(values, 3)
    set(values, 3)
    changed is in Byte: get(values, 3)
    return when first = 7 and last = 17 and changed = 19 then 0 else 1
"#,
        false,
    );
}

#[test]
fn dynamic_read_at_capacity_traps() {
    run(
        "#entry\nmain:\n    values Buffer(Byte, 4): [7; 4]\n    return get(values, 4)\n",
        true,
    );
}

#[test]
fn dynamic_negative_read_traps() {
    run(
        "#entry\nmain:\n    values Buffer(Byte, 4): [7; 4]\n    return get(values, -1)\n",
        true,
    );
}

#[test]
fn dynamic_write_at_capacity_traps() {
    run(
        "#entry\nmain:\n    values Buffer(Byte, 4): [7; 4]\n    set(values, 4)\n    return 0\n",
        true,
    );
}

#[test]
fn wide_index_is_checked_before_pointer_width_truncation() {
    run(
        "#entry\nmain:\n    values Buffer(Byte, 4): [7; 4]\n    return get_wide(values, 18446744073709551616)\n",
        true,
    );
}

#[test]
fn zero_capacity_rejects_a_dynamic_index() {
    run(
        "get_empty(values ref Buffer(Byte, 0), index Index) Byte: return values.[index]\n#entry\nmain:\n    values Buffer(Byte, 0): [7; 0]\n    return get_empty(values, 0)\n",
        true,
    );
}

#[test]
fn a_scalar_reference_accepts_only_its_single_element() {
    run(
        "get_one(value ref Byte, index Index) Byte: return value.[index]\n#entry\nmain:\n    value Byte: 7\n    observed is in Byte: get_one(value, 0)\n    return when observed = 7 then 0 else 1\n",
        false,
    );
    run(
        "get_one(value ref Byte, index Index) Byte: return value.[index]\n#entry\nmain:\n    value Byte: 7\n    return get_one(value, 1)\n",
        true,
    );
}
