//! A sum whose members carry payloads travels as its tag then the widest payload, shorter
//! members padded with zeros, and a payload field is read from the registers after the tag.
use std::path::Path;
use std::process::Command;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn package(main: &str) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "mod.flask",
        "#settings\nmodule: \"payloads\"\nversion: 0.1.0\n",
    );
    write(directory.path(), "main.gin", main);
    directory
}

fn mlir(main: &str) -> String {
    let directory = package(main);
    let output = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .arg("--emit")
        .arg("mlir")
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn run(main: &str) -> Option<i32> {
    let directory = package(main);
    let program = directory.path().join("program");
    let built = Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "exe", "--output"])
        .arg(&program)
        .arg(directory.path())
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    Command::new(&program).status().unwrap().code()
}

// An `or` name declares the union's own variant, so payload types are wrapped explicitly.
// The records and unions are `#unrestricted` to permit copying in these runtime tests.
const SAME_WIDTH: &str = "#unrestricted
SmallValue has value in 0 to 100
#unrestricted
BigValue has value in 0 to 100
#unrestricted
Number is Small(SmallValue) or Big(BigValue)

weigh(number Number) in 0 to 200:
    return when number is
        Small(held) then held.value
        Big(held) then held.value + 100

pick(n in 0 to 100) Number:
    return when n < 50 then Number.Small(SmallValue(value: n)) else Number.Big(BigValue(value: n))

relay(number Number) Number:
    return number

#entry
start() in 0 to 255: return weigh(relay(pick(70)))
";

const MIXED_WIDTH: &str = "#unrestricted
SingleValue has value in 0 to 100
#unrestricted
PairValue has first in 0 to 100, second in 0 to 100
#unrestricted
Shape is Single(SingleValue) or Pair(PairValue)

total(shape Shape) in 0 to 200:
    return when shape is
        Single(held) then held.value
        Pair(held) then held.first + held.second

make(n in 0 to 100) Shape:
    return when n < 50 then Shape.Single(SingleValue(value: n)) else Shape.Pair(PairValue(first: n, second: 3))

#entry
start() in 0 to 255: return total(make(70))
";

#[test]
fn a_sum_with_payloads_is_its_tag_and_the_payload_registers() {
    let text = mlir(SAME_WIDTH);
    // Tag and one payload register in, one number out.
    assert!(
        text.contains("@\"payloads.weigh\"(%arg0_0: i64, %arg0_1: i64) -> i64"),
        "{text}"
    );
    // A member becomes the sum at the return of `pick`.
    assert!(
        text.contains("@\"payloads.pick\"(%arg0: i64) -> (i64, i64)"),
        "{text}"
    );
}

#[test]
fn members_of_different_width_pad_to_the_widest() {
    let text = mlir(MIXED_WIDTH);
    assert!(
        text.contains("@\"payloads.total\"(%arg0_0: i64, %arg0_1: i64, %arg0_2: i64) -> i64"),
        "{text}"
    );
    assert!(text.contains("-> (i64, i64, i64)"), "{text}");
    // `Single` carries one register and so is padded with a zero.
    assert!(text.contains("arith.constant 0 : i64"), "{text}");
}

#[test]
fn the_payload_the_member_holds_is_read() {
    assert_eq!(run(SAME_WIDTH), Some(170));
    assert_eq!(run(MIXED_WIDTH), Some(73));
}
