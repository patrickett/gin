mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

fn run(source: &str, expected: i32) {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("terminal_cleanup");
        package.write_flask("terminal_cleanup");
        let input = package.write("main.gin", source);
        let executable = package.join("program");
        let output = compile_and_run_exe(input, &executable, profile, "", "run terminal cleanup");
        assert_eq!(output.status.code(), Some(expected));
    }
}

const LEAF: &str = r#"Word is in 0 to 255
Word has
    finish(self):
        return
    replace(previous Self, next Self) Self: return next
Leaf has
    count @Word
    last @Word
    id Word
    release(self):
        pointer: self.count
        previous is in Word: pointer.[0]
        pointer.[0]:: Word.replace(pointer.[0], ((previous + 1) & 255) as Word)
        last: self.last
        last.[0]:: Word.replace(last.[0], self.id)
        return
copy_pointer(pointer ref @Word) @Word: return deref pointer
"#;

#[test]
fn record_and_union_payload_cleanup_needs_no_wrapper_methods() {
    run(
        &format!(
            r#"{LEAF}
Pair has
    first Leaf
    second Leaf
Choice is Present(Pair) or Absent
finish(value Choice):
    when value is Present(pair) then
        pair.first.release
        pair.second.release
#entry
main:
    count Word: 0
    last Word: 0
    value: Present(Pair(first: Leaf(count: @count, last: @last, id: 1), second: Leaf(count: @count, last: @last, id: 2)))
    finish(value)
    empty Choice: Absent
    finish(empty)
    calls is in Word: count
    last_id is in Word: last
    return (((calls * 10) + last_id) & 255) as Word
"#
        ),
        22,
    );
}

#[test]
fn list_pattern_cleanup_releases_each_element_once() {
    for pattern in ["[head, ...tail]", "[head, second]"] {
        let body = if pattern.contains("tail") {
            "finish_parts(head, tail)"
        } else {
            "finish_pair(head, second)"
        };
        run(
            &format!(
                r#"{LEAF}
Bundle has
    items [Leaf; 2]
    release(self): return when self.items is {pattern} then {body}
finish_parts(head Leaf, tail [Leaf; 1]):
    head.release
    tail.[0].release
    return
finish_pair(head Leaf, second Leaf):
    head.release
    second.release
    return
#entry
main:
    count Word: 0
    last Word: 0
    value: Bundle(items: [Leaf(count: @count, last: @last, id: 1), Leaf(count: @count, last: @last, id: 2)])
    value.release
    calls is in Word: count
    last_id is in Word: last
    return (((calls * 10) + last_id) & 255) as Word
"#
            ),
            22,
        );
    }
}

#[test]
fn borrowed_pattern_payloads_observe_and_mutate_the_original_owner() {
    run(
        &format!(
            r#"{LEAF}
Maybe is Present(Leaf) or Absent
Maybe.release(self): return when self is Present(leaf) then leaf.release else idle
read(value ref Maybe) Word: return when value is Present(leaf) then leaf.id else 0 as Word
rename(value mut Maybe, next Word):
    when value is Present(leaf) then
        leaf.id:: Word.replace(leaf.id, next)
        return
    return
idle:
    return
#entry
main:
    count Word: 0
    last Word: 0
    value: Present(Leaf(count: @count, last: @last, id: 1))
    seen: read(value)
    rename(value, 2)
    value.release
    calls is in Word: count
    last_id is in Word: last
    original_id is in Word: seen
    return (((calls * 10) + last_id + original_id) & 255) as Word
"#
        ),
        13,
    );
}

#[test]
fn borrowed_list_patterns_preserve_element_addresses() {
    run(
        &format!(
            r#"{LEAF}
read(value ref [Leaf; 2]) Word:
    result: when value is [head, ...tail] then head.id
    return result
rename(value mut [Leaf; 2], next Word):
    when value is [head, ...tail] then
        tail.[0].id:: Word.replace(tail.[0].id, next)
        return
    return
finish(values [Leaf; 2]):
    values.[0].release
    values.[1].release
    return
#entry
main:
    count Word: 0
    last Word: 0
    values [Leaf; 2]: [Leaf(count: @count, last: @last, id: 1), Leaf(count: @count, last: @last, id: 2)]
    seen: read(values)
    rename(values, 3)
    finish(values)
    calls is in Word: count
    last_id is in Word: last
    original_id is in Word: seen
    return (((calls * 10) + last_id + original_id) & 255) as Word
"#
        ),
        24,
    );
}

#[test]
fn list_patterns_check_lengths_and_literal_elements() {
    run(
        &format!(
            r#"{LEAF}
classify(values ref [Word; 2]) Word: return when values is
    [] then 1 as Word
    [9, ..._] then 2 as Word
    [1] then 3 as Word
    [1, 2, extra] then deref extra
    [1, 2] then 5 as Word
    else 6 as Word
is_empty(value ref [Word; 0]) Word:
    result: when value is [] then 1 as Word
    return result
#entry
main:
    first [Word; 2]: [1, 2]
    second [Word; 2]: [1, 3]
    empty [Word; 0]: []
    left: classify(first)
    right: classify(second)
    empty_result: is_empty(empty)
    first.[0].finish
    first.[1].finish
    second.[0].finish
    second.[1].finish
    left_code is in Word: left
    right_code is in Word: right
    empty_code is in Word: empty_result
    return (((left_code * 10) + right_code + empty_code) & 255) as Word
"#
        ),
        57,
    );
}

#[test]
fn explicit_field_cleanup_preserves_source_order() {
    for (body, expected) in [
        ("self.second.release\n        self.first.release", 1),
        ("self.first.release\n        self.second.release", 2),
    ] {
        run(
            &format!(
                r#"{LEAF}
Pair has
    first Leaf
    second Leaf
    release(self):
        {body}
        return
#entry
main:
    count Word: 0
    last Word: 0
    value: Pair(first: Leaf(count: @count, last: @last, id: 1), second: Leaf(count: @count, last: @last, id: 2))
    value.release
    return last
"#
            ),
            expected,
        );
    }
}

#[test]
fn transferred_field_is_released_only_by_its_new_owner() {
    run(
        &format!(
            r#"{LEAF}
Pair has
    first Leaf
    second Leaf
    take(self) Leaf:
        self.second.release
        return self.first
#entry
main:
    count Word: 0
    last Word: 0
    value: Pair(first: Leaf(count: @count, last: @last, id: 1), second: Leaf(count: @count, last: @last, id: 2))
    leaf: value.take
    leaf.release
    return count
"#
        ),
        2,
    );
}

#[test]
fn borrowed_fields_and_scope_exit_do_not_release_resources() {
    run(
        &format!(
            r#"{LEAF}
Borrower has
    leaf ref Leaf
    finish(self):
        return
exercise(count ref @Word, last ref @Word):
    leaf: Leaf(count: copy_pointer(count), last: copy_pointer(last), id: 1)
    borrower: Borrower(leaf: leaf)
    borrower.finish
    return
#entry
main:
    count Word: 0
    last Word: 0
    count_pointer: (@count) as @Word
    last_pointer: (@last) as @Word
    exercise(count_pointer, last_pointer)
    return count
"#
        ),
        0,
    );
}

#[test]
fn conditional_payload_cleanup_runs_once() {
    run(
        &format!(
            r#"{LEAF}
Choice is Present(Leaf) or Absent
Choice has
    finish(self) Choice:
        when self is Present(leaf) then
            leaf.release
            return Absent
        return self
#entry
main:
    count Word: 0
    last Word: 0
    value Choice: Present(Leaf(count: @count, last: @last, id: 7))
    remaining: value.finish
    when remaining is Absent then idle else idle
    return count
idle:
    return
"#
        ),
        1,
    );
}

#[test]
fn loop_early_returns_cleanup_each_field_once() {
    for (flag, expected) in [(0, 22), (1, 21)] {
        run(
            &format!(
                r#"{LEAF}
Pair has
    first Leaf
    second Leaf
    finish(self, flag ref Word):
        while flag is 1 do
            when flag is 1 then
                self.second.release
                self.first.release
                return
        self.first.release
        self.second.release
        return
#entry
main:
    count Word: 0
    last Word: 0
    flag Word: {flag}
    value: Pair(first: Leaf(count: @count, last: @last, id: 1), second: Leaf(count: @count, last: @last, id: 2))
    value.finish(flag)
    calls is in Word: count
    last_id is in Word: last
    return ((calls * 10 + last_id) & 255) as Word
"#
            ),
            expected,
        );
    }
}
