//! Shapes the recorder reproduces so that a program lowers as the checker proved it: a
//! membership test whose upper bound the operand's interval already meets (a domain wider than
//! the machine word), an `#unrestricted` domain read as the number a result `in D` holds, a
//! choice between members of a sum whose arms change a local, and a `mut` call on an owned
//! record parameter. Each is built and run natively, with a case on either side of its choice.
use std::path::Path;
use std::process::Command;

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"conversions\"\nversion: 0.1.0\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.gin"), main).unwrap();
    directory
}

fn build(main: &str) -> (tempfile::TempDir, std::process::Output) {
    let directory = package(main);
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    (directory, built)
}

fn run(main: &str) -> Option<i32> {
    let (directory, built) = build(main);
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    Command::new(Path::new(directory.path()).join("program"))
        .status()
        .unwrap()
        .code()
}

/// `value is in Size` over a signed word: the upper bound (2^64 - 1) is past every word, so
/// only the lower bound is compared.
fn membership(argument: &str) -> String {
    format!(
        "#unrestricted
Size is in 0 to 18446744073709551615

fits(value in -9223372036854775808 to 9223372036854775807) in 0 to 1:
    return when value is in Size then 1 else 0

#entry
main in 0 to 1:
    return fits({argument})
"
    )
}

#[test]
fn a_membership_wider_than_the_word_compares_only_its_lower_bound() {
    assert_eq!(run(&membership("5")), Some(1));
    assert_eq!(run(&membership("9223372036854775807")), Some(1));
    // Negative control: below the lower bound is still outside.
    assert_eq!(run(&membership("-1")), Some(0));
}

const OPEN_READ: &str = "#unrestricted
Code is in -100 to 100

#unrestricted
Holder has
    code Code

    read(self ref) in Code:
        return self.code

#entry
main in 0 to 100:
    holder: Holder(code: 42)
    value in Code: holder.read
    return when value < 0 then 0 else value
";

#[test]
fn an_open_domain_is_returned_as_the_number_a_result_in_it_holds() {
    assert_eq!(run(OPEN_READ), Some(42));
    // Negative control: a negative code takes the other arm.
    assert_eq!(run(&OPEN_READ.replace("code: 42", "code: -7")), Some(0));
}

const CHANGED_ARMS: &str = "#unrestricted
Counter has
    count in 0 to 100

    bump(self mut) in 0 to 100:
        self.count :: when self.count < 100 then self.count + 1 else self.count
        return self.count

#unrestricted
Count is in 0 to 110

#unrestricted
Outcome is Counted(Count) or Refused

#unrestricted
Box has
    counter Counter

    step(self mut, code in -5 to 5) Outcome:
        return when code < 0 then Refused else (when code = 1 then Counted(self.counter.bump) else Counted(self.counter.bump + 10))

score(outcome Outcome) in 0 to 200: return when outcome is
    Counted(count) then count
    Refused then 150

#entry
main in 0 to 200:
    box: Box(counter: Counter(count: 3))
    first: box.step(CODE)
    return score(first)
";

#[test]
fn a_choice_between_members_changes_a_local_in_its_arms() {
    assert_eq!(run(&CHANGED_ARMS.replace("CODE", "1")), Some(4));
    assert_eq!(run(&CHANGED_ARMS.replace("CODE", "2")), Some(14));
    // Negative control: the arm that changes nothing.
    assert_eq!(run(&CHANGED_ARMS.replace("CODE", "-1")), Some(150));
}

const OWNED_MUT: &str = "#unrestricted
Pen has
    column in 0 to 50

    right(self mut):
        self.column :: when self.column < 50 then self.column + 1 else self.column
        return

    finish(pen Pen) in 0 to 50:
        pen.right
        pen.right
        return pen.column

#entry
main in 0 to 50:
    return Pen.finish(Pen(column: START))
";

#[test]
fn a_mut_call_rebinds_an_owned_record_parameter() {
    assert_eq!(run(&OWNED_MUT.replace("START", "3")), Some(5));
    // Negative control: at the edge the method leaves the column alone.
    assert_eq!(run(&OWNED_MUT.replace("START", "50")), Some(50));
}

/// A program over the core library, built for the host; its standard output and status.
fn run_with_core(main: &str) -> (String, Option<i32>) {
    let directory = tempfile::tempdir().unwrap();
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    std::os::unix::fs::symlink(core.canonicalize().unwrap(), directory.path().join("core"))
        .unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"with_core\"\nversion: 0.1.0\n\n#dependencies\ncore: \"./core\"\n",
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
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let done = Command::new(&program).output().unwrap();
    (
        String::from_utf8_lossy(&done.stdout).into_owned(),
        done.status.code(),
    )
}

// `GlobalAllocator.reserve` narrows `count * width` after `when requested > max_extent then
// return`: the narrowed value is the fall-through's, and was once placed in the guard's arm
// (an SSA value used where it was not defined) for elements wider than a byte.
const WORD_LIST: &str = "use core.collection List, ListFailure
use core.alloc GlobalAllocator
use core.io print_int
use core.primitive SignedBigInt

#entry
main in 0 to 1:
    attempt: List.with_capacity(SignedBigInt, GlobalAllocator, CAPACITY, GlobalAllocator())
    when attempt is Failed(failure) then
        failure.release
        return 1
    Allocated(numbers) := attempt
    numbers.push(7 as SignedBigInt)
    numbers.push(-9 as SignedBigInt)
    second: deref numbers.borrow(1)
    print_int(second)
    numbers.release
    return 0
";

#[test]
fn a_guard_narrows_the_value_the_rest_of_the_block_uses() {
    assert_eq!(
        run_with_core(&WORD_LIST.replace("CAPACITY", "4")),
        ("-9\n".to_owned(), Some(0))
    );
    // Negative control: a capacity past what can be mapped takes the guard's own exit.
    assert_eq!(
        run_with_core(
            &WORD_LIST
                .replace("CAPACITY", "4611686018427387903")
                .replace("    numbers.push(-9 as SignedBigInt)\n", "")
                .replace(
                    "    second: deref numbers.borrow(1)\n    print_int(second)\n",
                    ""
                )
        ),
        (String::new(), Some(1))
    );
}
