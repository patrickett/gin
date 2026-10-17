mod common;
use common::{compile_and_run_exe, compile_exe};

use ginc::cli::Profile;
use ginc::compile::{CompileFailure, CompileResult};
use test_fixtures::TempPackage;

const PRELUDE: &str = r#"
Word is in 0 to 255
Word has finish(self): return
replace(previous Word, next Word) Word: return next
Slice(element) has
    length Word
    data ref [element; length]
    finish(self):
        return
"#;

fn run(name: &str, source: &str, expected: Option<i32>) {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new(name);
        package.write_flask(name);
        let output = compile_and_run_exe(
            package.write("main.gin", &format!("{PRELUDE}{source}")),
            &package.join("program"),
            profile,
            "",
            "run dependent sequence reference",
        );
        match expected {
            Some(code) => assert_eq!(output.status.code(), Some(code), "{name} {profile:?}"),
            None => {
                assert!(!output.status.success(), "{name} {profile:?}");
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    assert!(
                        matches!(output.status.signal(), Some(4 | 5)),
                        "{name} {profile:?}: {}",
                        output.status
                    );
                }
            }
        }
    }
}

#[test]
fn dependent_reference_indexing_reads_the_element() {
    run(
        "dependent_index",
        r#"
peek(n Word, source ref [Word; n], index Word) Word: return source.[index]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    return peek(4, source, 2)
"#,
        Some(30),
    );
}

#[test]
fn dependent_reference_indexing_traps_at_the_length() {
    run(
        "dependent_index_trap",
        r#"
peek(n Word, source ref [Word; n], index Word) Word: return source.[index]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    return peek(4, source, 4)
"#,
        None,
    );
}

#[test]
fn dependent_reference_iteration_visits_each_element() {
    run(
        "dependent_iteration",
        r#"
sum(n Word, source ref [Word; n]) Word:
    total Word: 0
    for value in source
        amount is in 0 to 255: value
        previous is in 0 to 255: total
        total:: replace(total, ((previous + amount) & 255) as Word)
    return total
#entry
main:
    source [Word; 4]: [1, 2, 3, 4]
    return sum(4, source)
"#,
        Some(10),
    );
}

#[test]
fn dependent_record_indexing_reads_the_element() {
    run(
        "dependent_record_index",
        r#"
peek(slice ref Slice(Word), index Word) Word: return slice.data.[index]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    slice: Slice(length: 4 as Word, data: source)
    result: peek(slice, 2)
    slice.finish
    return result
"#,
        Some(30),
    );
}

#[test]
fn dependent_reference_iteration_handles_empty_storage() {
    run(
        "dependent_empty_iteration",
        r#"
count(n Word, source ref [Word; n]) Word:
    total Word: 17
    for value in source
        total:: replace(total, 99)
    return total
#entry
main:
    source [Word; 0]: []
    return count(0, source)
"#,
        Some(17),
    );
}

#[test]
fn dependent_mutable_iteration_updates_backing_storage() {
    run(
        "dependent_mutable_iteration",
        r#"
set(value mut Word):
    value:: replace(deref value, 7)
    return
update(n Word, source mut [Word; n]):
    for value in source
        set(value)
    return
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    update(4, source)
    return source.[2]
"#,
        Some(7),
    );
}

#[test]
fn dependent_reference_indexing_traps_for_a_negative_index() {
    run(
        "dependent_negative_index",
        r#"
Offset is in -128 to 127
Offset has finish(self): return
peek(n Word, source ref [Word; n], index Offset) Word: return source.[index]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    return peek(4, source, -1)
"#,
        None,
    );
}

#[test]
fn indexing_a_returned_dependent_record_evaluates_the_source_once() {
    run(
        "dependent_temporary_record",
        r#"
make(source ref [Word; 4], calls mut Word) Slice(Word):
    previous is in Word: calls
    calls:: replace(deref calls, ((previous + 1) & 255) as Word)
    return Slice(length: 4 as Word, data: source)
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    calls Word: 0
    result: make(source, calls).data.[1]
    count is in Word: calls
    amount is in Word: result
    return count + amount
"#,
        Some(21),
    );
}

#[test]
fn an_oversized_dependent_record_stops_binary_emission() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("dependent_invalid_extent");
        package.write_flask("dependent_invalid_extent");
        let source = format!(
            "{PRELUDE}{}",
            r#"
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    slice: Slice(length: 5 as Word, data: source)
    slice.finish
    return 0
"#
        );
        let output = package.join("program");
        assert!(matches!(
            compile_exe(package.write("main.gin", &source), &output, profile),
            CompileResult::Failed(CompileFailure::TypecheckFailed)
        ));
        assert!(!output.exists());
    }
}

#[test]
fn a_dependent_prefix_borrow_indexes_the_selected_storage() {
    run(
        "dependent_slice_prefix",
        r#"
prefix(n Word, source ref [Word; 4]) ref [Word; n]: return ref source.[1; n]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    view: prefix(2, source)
    return view.[1]
"#,
        Some(30),
    );
}

#[test]
fn a_dependent_prefix_borrow_iterates_the_requested_count() {
    run(
        "dependent_slice_iteration",
        r#"
sum(n Word, source ref [Word; 4]) Word:
    view: ref source.[1; n]
    total Word: 0
    for value in view
        amount is in Word: value
        previous is in Word: total
        total:: replace(total, ((previous + amount) & 255) as Word)
    return total
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    return sum(2, source)
"#,
        Some(50),
    );
}

#[test]
fn a_dependent_prefix_borrow_checks_the_backing_extent() {
    run(
        "dependent_slice_bounds",
        r#"
prefix(n Word, source ref [Word; 4]) ref [Word; n]: return ref source.[1; n]
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    view: prefix(4, source)
    return view.[0]
"#,
        None,
    );
}

#[test]
fn a_mutable_prefix_borrow_updates_only_the_selected_storage() {
    run(
        "dependent_slice_mutable",
        r#"
set(value mut Word):
    value:: replace(deref value, 7)
    return
update(n Word, source mut [Word; 4]):
    view: mut source.[1; n]
    for value in view
        set(value)
    return
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    update(2, source)
    return source.[2]
"#,
        Some(7),
    );
}

#[test]
fn a_subslice_uses_the_borrowed_extent_and_offset() {
    for (length, expected) in [(1, Some(30)), (2, None)] {
        run(
            "dependent_subslice",
            &format!(
                "choose(n Word) Word:\n    source [Word; 4]: [10, 20, 30, 40]\n    first: ref source.[1; 2]\n    second: ref first.[1; n]\n    return second.[0]\n#entry\nmain: choose({length})\n"
            ),
            expected,
        );
    }
}

#[test]
fn slice_borrow_operands_are_evaluated_once_in_order() {
    run(
        "dependent_slice_evaluation",
        r#"
record(calls mut Word, value Word) Word:
    previous is in Word: calls
    amount is in Word: value
    calls:: replace(deref calls, ((previous * 10 + amount) & 255) as Word)
    return value
source(bytes ref{storage} [Word; 4], calls mut{counter} Word) ref{storage} [Word; 4]:
    record(calls, 1)
    return bytes
#entry
main:
    bytes [Word; 4]: [10, 20, 30, 40]
    calls Word: 0
    view: ref source(bytes, calls).[record(calls, 2); record(calls, 1)]
    result: view.[0]
    order is in Word: calls
    amount is in Word: result
    return when order is 121 then amount else 99
"#,
        Some(30),
    );
}

#[test]
fn an_empty_tail_slice_skips_iteration() {
    run(
        "dependent_slice_empty_tail",
        r#"
visit(n Word, source ref [Word; 4]) Word:
    view: ref source.[4; n]
    total Word: 17
    for value in view
        total:: replace(total, 99)
    return total
#entry
main:
    source [Word; 4]: [10, 20, 30, 40]
    return visit(0, source)
"#,
        Some(17),
    );
}

#[test]
fn invalid_slice_lifetimes_and_permissions_stop_binary_emission() {
    for source in [
        r#"
escape(seed Word) ref [Word; 2]:
    values [Word; 4]: [seed, 20, 30, 40]
    return ref values.[0; 2]
#entry
main:
    view: escape(10)
    return view.[0]
"#,
        r#"
consume(values [Word; 4]): return
#entry
main:
    values [Word; 4]: [10, 20, 30, 40]
    view: ref values.[0; 2]
    consume(values)
    return view.[0]
"#,
        r#"
consume(values [Word; 4]): return
skip(value Word): return
check(flag Word) Word:
    values [Word; 4]: [10, 20, 30, 40]
    view: ref values.[0; 2]
    when flag is 1 then consume(values) else skip(0)
    return view.[0]
#entry
main: check(1)
"#,
        r#"
View has
    len Word
    data mut [Word; len]
    finish(self): return
expose(view ref View) mut Word: return mut view.data.[0]
#entry
main: 0
"#,
        r#"
View has
    len Word
    data mut [Word; len]
    finish(self): return
duplicate(view ref View) View: return deref view
#entry
main: 0
"#,
    ] {
        for profile in [Profile::Debug, Profile::Release] {
            let package = TempPackage::new("dependent_slice_invalid_lifetime");
            package.write_flask("dependent_slice_invalid_lifetime");
            let output = package.join("program");
            assert!(matches!(
                compile_exe(
                    package.write("main.gin", &format!("{PRELUDE}{source}")),
                    &output,
                    profile
                ),
                CompileResult::Failed(CompileFailure::TypecheckFailed)
            ));
            assert!(!output.exists());
        }
    }
}

#[test]
fn promoted_literal_arrays_outlive_their_creator() {
    run(
        "promoted_literal_array",
        r#"
literal(seed Word) ref [Word; 2]: return ref [10 as Word, 20 as Word]
#entry
main:
    source: literal(0)
    return source.[1]
"#,
        Some(20),
    );
}

#[test]
fn promoted_repeated_arrays_outlive_their_creator() {
    run(
        "promoted_repeated_array",
        r#"
literal(seed Word) ref [Word; 2]: return ref [17 as Word; 2]
#entry
main:
    source: literal(0)
    return source.[1]
"#,
        Some(17),
    );
}

#[test]
fn promoted_arrays_support_nested_storage_and_subslices() {
    run(
        "promoted_nested_array",
        r#"
literal(seed Word) ref [[Word; 2]; 2]: return ref [[10 as Word, 20 as Word], [30 as Word, 40 as Word]]
#entry
main:
    source: literal(0)
    return source.[1].[0]
"#,
        Some(30),
    );
    run(
        "promoted_array_subslice",
        r#"
literal(seed Word) ref [Word; 2]: return ref [10 as Word, 20 as Word]
#entry
main:
    source: literal(0)
    data: ref source.[1; 1]
    return data.[0]
"#,
        Some(20),
    );
}

#[test]
fn promoted_arrays_remain_static_through_ordinary_records() {
    run(
        "promoted_record_array",
        r#"
literal(seed Word) Slice(Word):
    data: ref [10 as Word, 20 as Word]
    return Slice(length: 2 as Word, data: data)
forward(seed Word) Slice(Word): return literal(seed)
#entry
main:
    source: forward(0)
    answer Word: source.data.[1]
    source.finish
    return answer
"#,
        Some(20),
    );
}

#[test]
fn promoted_empty_arrays_can_be_returned_and_iterated() {
    run(
        "promoted_empty_array",
        r#"
literal(seed Word) ref [Word; 0]: return ref []
#entry
main:
    source: literal(0)
    total Word: 7
    for value in source
        total:: replace(total, 99)
    return total
"#,
        Some(7),
    );
}
