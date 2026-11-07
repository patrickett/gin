//! Library behaviour that must hold natively on the host, and bodies the recorder must refuse
//! rather than lower wrongly.
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build `main` against the core library; the compiler's stderr and the program path.
fn build(name: &str, main: &str) -> (tempfile::TempDir, PathBuf, std::process::Output) {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        format!("#settings\nmodule: \"{name}\"\nversion: 0.1.0\ntarget: \"aarch64-apple-darwin\"\n\n#dependencies\ncore: \"./core\"\n"),
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    (directory, program, built)
}

fn run(program: &Path) -> String {
    let done = Command::new(program).output().unwrap();
    String::from_utf8_lossy(&done.stdout).into_owned()
}

const HEAD: &str = "use core.collection List, ListFailure\nuse core.alloc GlobalAllocator\nuse core.slice Byte, SliceView\nuse core.io print_int\nuse core.primitive SignedBigInt\n\n";

#[test]
fn replace_hands_back_the_previous_element_and_stores_the_new_one() {
    let main = format!(
        "{HEAD}show(items ref List(Byte, GlobalAllocator) and items.length > 1):\n    seen: items.view\n    got: seen.borrow(1)\n    print_int(got.as_num as SignedBigInt)\n    return\n\n#entry\nmain:\n    attempt: List.with_capacity(Byte, GlobalAllocator, 3, GlobalAllocator())\n    when attempt is Allocated(items) then\n        items.push(7 as Byte)\n        items.push(8 as Byte)\n        old: items.replace(1, 9 as Byte)\n        print_int(old.as_num as SignedBigInt)\n        show(items)\n        items.release\n        return\n    Failed(failure) := attempt\n    failure.release\n    return\n"
    );
    let (_directory, program, built) = build("replace_case", &main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(run(&program), "8\n9\n");
    // Negative control: an index past the length is not proved.
    let past = main.replace("items.replace(1,", "items.replace(2,");
    let (_d, _p, built) = build("replace_past", &past);
    assert!(!built.status.success());
    // Negative control: the previous element (a linear one in general) cannot be dropped silently
    // is covered for numbers by being unrestricted; a missing receiver `mut` is refused.
}

#[test]
fn a_push_in_one_arm_of_a_conditional_is_kept() {
    // It once lowered to a function that returned the list unchanged (length 0), then was
    // declined; each arm now hands back its own list.
    let main = r#"use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.slice Byte, SliceView
use core.io print_int
use core.primitive SignedBigInt

put(items mut List(Byte, GlobalAllocator), value Byte):
    when items.length < items.capacity
        then items.push(value)
        else value.release
    return

#entry
main:
    attempt: List.with_capacity(Byte, GlobalAllocator, 2, GlobalAllocator())
    when attempt is Allocated(items) then
        put(items, 4 as Byte)
        print_int(items.length as SignedBigInt)
        put(items, 5 as Byte)
        put(items, 6 as Byte)
        print_int(items.length as SignedBigInt)
        items.release
        return
    Failed(failure) := attempt
    failure.release
    return
"#;
    let (_directory, program, built) = build("push_in_arm", main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    // Past capacity the value is released and the list keeps its two elements.
    assert_eq!(run(&program), "1\n2\n");
    // Negative control: an arm that drops the value it does not push leaks it.
    let leak = main.replace(
        "        else value.release\n",
        "        else value.as_num\n",
    );
    let (_d, _p, built) = build("push_in_arm_leak", &leak);
    assert!(!built.status.success(), "a leaked value must be refused");
}

#[test]
fn a_guard_with_an_early_failure_return_hands_back_the_mutated_list() {
    // The mutation (two pushes and a `mut` argument to `join`) happens on one path only; the
    // `Failed` path returns the list as given. This used to emit MLIR that did not verify.
    let main = r#"use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.slice Byte, SliceView
use core.io print_int
use core.primitive SignedBigInt

show(items ref List(Byte, GlobalAllocator) and items.length = 4):
    seen: items.view
    for index in 0 to 3 do
        got: seen.borrow(index)
        print_int(got.as_num as SignedBigInt)
    return

join(left mut List(Byte, GlobalAllocator) and left.length = 2 and left.capacity = 4, right List(Byte, GlobalAllocator) and right.length = 2):
    left.append(right)
    show(left)
    return

with_right(left mut List(Byte, GlobalAllocator) and left.length = 0 and left.capacity = 4, a Byte, b Byte, c Byte, d Byte):
    second: List.with_capacity(Byte, GlobalAllocator, 2, GlobalAllocator())
    when second is Allocated(right) then
        left.push(a)
        left.push(b)
        right.push(c)
        right.push(d)
        join(left, right)
        return
    Failed(failure) := second
    failure.release
    return

#entry
main:
    first: List.with_capacity(Byte, GlobalAllocator, 4, GlobalAllocator())
    when first is Allocated(left) then
        with_right(left, 1 as Byte, 2 as Byte, 3 as Byte, 4 as Byte)
        left.release
        return
    Failed(failure) := first
    failure.release
    return
"#;
    let (_directory, program, built) = build("guard_failed_path", main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(run(&program), "1\n2\n3\n4\n");
    // Negative control: the failure that is not released is refused.
    let leak = main.replacen(
        "    failure.release\n    return\n\n#entry",
        "    return\n\n#entry",
        1,
    );
    assert_ne!(leak, main);
    let (_d, _p, built) = build("guard_failed_leak", &leak);
    assert!(
        !built.status.success(),
        "an unreleased failure must be refused"
    );
}

#[test]
fn exits_that_mutate_different_outputs_or_none_hand_back_each_path() {
    let main = r#"use core.io print_int
use core.primitive SignedBigInt

bump(n mut in 0 to 20):
    when n < 20 then
        n :: n + 1
        return
    return

pick(a mut in 0 to 20, b mut in 0 to 20, c in 0 to 10):
    when c < 5 then
        when a < 20 then
            a :: a + 1
            return
        return
    when b < 19 then
        b :: b + 2
        return
    return

maybe(a mut in 0 to 20, c in 0 to 10):
    when c < 5 then
        return
    bump(a)
    return

#entry
main:
    x in 0 to 20: 1
    y in 0 to 20: 1
    pick(x, y, 3)
    print_int(x as SignedBigInt)
    print_int(y as SignedBigInt)
    pick(x, y, 7)
    print_int(x as SignedBigInt)
    print_int(y as SignedBigInt)
    z in 0 to 20: 4
    maybe(z, 2)
    print_int(z as SignedBigInt)
    maybe(z, 9)
    print_int(z as SignedBigInt)
    return
"#;
    let (_directory, program, built) = build("per_exit_outputs", main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    // pick(1, 1, 3) bumps x; pick(2, 1, 7) bumps y by 2; maybe(4, 2) changes nothing; maybe(4, 9)
    // bumps z through a `mut` argument.
    assert_eq!(run(&program), "2\n1\n2\n3\n4\n5\n");
    // Negative control: an output outside its declared range on one path is refused.
    let wide = main.replace("    when b < 19 then\n", "    when b < 20 then\n");
    let (_d, _p, built) = build("per_exit_outputs_range", &wide);
    assert!(
        !built.status.success(),
        "an output past its range must be refused"
    );
}

#[test]
fn a_loop_on_one_path_hands_back_its_result() {
    let main = r#"use core.io print_int
use core.primitive SignedBigInt

spin(a mut in 0 to 30, n in 0 to 5):
    when n < 2 then
        return
    for i in 1 to 3 do
        a :: when a < 30 then a + 1 else a
    return

#entry
main:
    x in 0 to 30: 5
    spin(x, 1)
    print_int(x as SignedBigInt)
    spin(x, 4)
    print_int(x as SignedBigInt)
    return
"#;
    let (_directory, program, built) = build("per_exit_loop", main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(run(&program), "5\n8\n");
}

/// Two lists joined: `left` (capacity 4) takes `right` (2 elements) after its own 2.
const APPEND: &str = "use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.slice Byte, SliceView
use core.io print_int
use core.primitive SignedBigInt

show(items ref List(Byte, GlobalAllocator) and items.length = 4):
    seen: items.view
    for index in 0 to 3 do
        got: seen.borrow(index)
        print_int(got.as_num as SignedBigInt)
    return

fill(left mut List(Byte, GlobalAllocator) and left.length = 0 and left.capacity = 4, right mut List(Byte, GlobalAllocator) and right.length = 0 and right.capacity = 2) where left.length :: 2, right.length :: 2, left.capacity :: 4, right.capacity :: 2:
    left.push(1 as Byte)
    left.push(2 as Byte)
    right.push(3 as Byte)
    right.push(4 as Byte)
    return

#entry
main:
    first: List.with_capacity(Byte, GlobalAllocator, 4, GlobalAllocator())
    when first is Allocated(left) then
        second: List.with_capacity(Byte, GlobalAllocator, 2, GlobalAllocator())
        when second is Allocated(right) then
            fill(left, right)
            left.append(right)
            show(left)
            left.release
            return
        Failed(failure) := second
        failure.release
        left.release
        return
    Failed(failure) := first
    failure.release
    return
";

#[test]
fn append_moves_the_other_lists_elements_in_order_onto_the_end() {
    let (_directory, program, built) = build("append_order", APPEND);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(run(&program), "1\n2\n3\n4\n");
}

#[test]
fn append_is_refused_past_capacity_after_use_and_without_release() {
    // Past capacity: the left list has room for 3, not 4.
    let small = APPEND.replace(
        "GlobalAllocator, 4, GlobalAllocator()",
        "GlobalAllocator, 3, GlobalAllocator()",
    );
    let (_d, _p, built) = build("append_past", &small);
    assert!(!built.status.success());
    // Double use: `right` was consumed by the append.
    let twice = APPEND.replace(
        "            left.append(right)\n",
        "            left.append(right)\n            right.release\n",
    );
    let (_d, _p, built) = build("append_twice", &twice);
    assert!(!built.status.success());
    // Leak: the joined list is never released.
    let leak = APPEND.replace(
        "            left.release\n            return\n        Failed(failure) := second",
        "            return\n        Failed(failure) := second",
    );
    let (_d, _p, built) = build("append_leak", &leak);
    assert!(!built.status.success());
}

#[test]
fn append_on_one_path_keeps_order_and_refuses_misuse() {
    // `left` is lent `mut` to `append` on the `Allocated` path only.
    let main = r#"use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.slice Byte, SliceView
use core.io print_int
use core.primitive SignedBigInt

show(items ref List(Byte, GlobalAllocator) and items.length = 3):
    seen: items.view
    for index in 0 to 2 do
        got: seen.borrow(index)
        print_int(got.as_num as SignedBigInt)
    return

fill(left mut List(Byte, GlobalAllocator) and left.length = 0 and left.capacity = 3):
    second: List.with_capacity(Byte, GlobalAllocator, 2, GlobalAllocator())
    when second is Allocated(right) then
        left.push(7 as Byte)
        right.push(8 as Byte)
        right.push(9 as Byte)
        left.append(right)
        show(left)
        return
    Failed(failure) := second
    failure.release
    return

#entry
main:
    first: List.with_capacity(Byte, GlobalAllocator, 3, GlobalAllocator())
    when first is Allocated(left) then
        fill(left)
        print_int(left.length as SignedBigInt)
        left.release
        return
    Failed(failure) := first
    failure.release
    return
"#;
    let (_directory, program, built) = build("append_one_path", main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(run(&program), "7\n8\n9\n3\n");
    // Negative controls: past capacity, `right` used after `append` took it, and a leaked list.
    let past = main
        .replace("left.capacity = 3", "left.capacity = 2")
        .replace(
            "GlobalAllocator, 3, GlobalAllocator()",
            "GlobalAllocator, 2, GlobalAllocator()",
        );
    let twice = main.replace(
        "        left.append(right)\n",
        "        left.append(right)\n        right.release\n",
    );
    let leak = main.replace("        left.release\n", "");
    for (name, bad) in [
        ("append_past", past),
        ("append_twice", twice),
        ("append_leak", leak),
    ] {
        assert_ne!(bad, main, "{name}");
        let (_d, _p, built) = build(name, &bad);
        assert!(!built.status.success(), "{name} must be refused");
    }
}
