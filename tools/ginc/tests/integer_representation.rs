//! Integer machine representations must preserve exact Gin values at register boundaries.
use std::io::Write;
use std::process::{Command, Stdio};

fn emit(source: &str) -> std::process::Output {
    let parsed = parser::parse_strict(source).expect("valid integer fixture syntax");
    typecheck::check_file(&parsed, typecheck::Limits::default())
        .expect("integer fixture checks before lowering");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("input.gin");
    std::fs::write(&path, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_ginc"))
        .args(["--emit", "mlir"])
        .arg(path)
        .env_remove("GIN_PROFILE_JSON")
        .env_remove("GINC_TIMINGS")
        .output()
        .unwrap()
}

fn pipe(program: &str, arguments: &[&str], input: &str) -> String {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{program}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn comparison_main(cases: &[(i64, u64, bool)]) -> String {
    assert!(!cases.is_empty());
    assert!(cases.len() <= 128);
    let mut main = String::from("module {\n  func.func @main() -> i32 {\n");
    main.push_str("    %zero = arith.constant 0 : i32\n");
    for (index, &(left, right, expected)) in cases.iter().enumerate() {
        let right = i64::from_ne_bytes(right.to_ne_bytes());
        let expected = u8::from(expected);
        main.push_str(&format!(
            "    %a{index} = arith.constant {left} : i64\n\
             \x20   %b{index} = arith.constant {right} : i64\n\
             \x20   %r{index} = func.call @compare(%a{index}, %b{index}) : (i64, i64) -> i64\n\
             \x20   %e{index} = arith.constant {expected} : i64\n\
             \x20   %bad{index} = arith.cmpi ne, %r{index}, %e{index} : i64\n\
             \x20   %err{index} = arith.extui %bad{index} : i1 to i32\n"
        ));
        let previous = if index == 0 {
            "%zero".to_owned()
        } else {
            format!("%sum{}", index - 1)
        };
        main.push_str(&format!(
            "    %sum{index} = arith.addi {previous}, %err{index} : i32\n"
        ));
    }
    main.push_str(&format!("    return %sum{} : i32\n  }}", cases.len() - 1));
    main
}

/// Call the emitted function from MLIR so Gin cannot fold its arguments before lowering.
fn run_comparisons(source: &str, cases: &[(i64, u64, bool)]) {
    let output = emit(source);
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let start = text.find("module {").unwrap();
    let end = text.rfind("\n}").unwrap() + 2;
    let module = text[start..end].replacen("module {", &comparison_main(cases), 1);
    let lowered = pipe(
        "mlir-opt",
        &[
            "--convert-scf-to-cf",
            "--convert-arith-to-llvm",
            "--convert-cf-to-llvm",
            "--convert-func-to-llvm",
            "--reconcile-unrealized-casts",
        ],
        &module,
    );
    let llvm = pipe("mlir-translate", &["--mlir-to-llvmir"], &lowered);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("program.ll");
    std::fs::write(&path, llvm).unwrap();
    let status = Command::new("lli").arg(path).status().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "at least one exact comparison disagrees"
    );
}

#[test]
fn mixed_signed_unsigned_equality_preserves_distinct_values_with_identical_bits() {
    let source = "\
compare(a in -1 to 0, b in 0 to 18446744073709551615) in 0 to 1:
    return when a = b then 1 else 0
";
    run_comparisons(source, &[(-1, u64::MAX, false), (0, 0, true)]);
}

#[test]
fn an_integer_signature_that_needs_more_than_one_register_is_refused() {
    for bounds in [
        "0 to 1208925819614629174706176",
        "-1 to 18446744073709551615",
        // Truncating this upper bound to i128 would turn it into zero.
        "0 to 340282366920938463463374607431768211456",
    ] {
        let source = format!("identity(x in {bounds}) in {bounds}: return x\n");
        let output = emit(&source);
        assert_eq!(output.status.code(), Some(1), "{source}: {output:?}");
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("func.func"),
            "{output:?}"
        );
    }
}

#[test]
fn all_mixed_comparisons_agree_with_exact_arithmetic_at_register_edges() {
    let left = [i64::MIN, -1, 0, 1, i64::MAX];
    let right = [0, 1, i64::MAX as u64, 1u64 << 63, u64::MAX];
    for operator in ["=", "<", "<=", ">", ">="] {
        let source = format!(
            "compare(a in -9223372036854775808 to 9223372036854775807, \
             b in 0 to 18446744073709551615) in 0 to 1:\n\
             \x20   return when a {operator} b then 1 else 0\n"
        );
        let mut cases = Vec::new();
        for a in left {
            for b in right {
                let exact = i128::from(a).cmp(&i128::from(b));
                let expected = match operator {
                    "=" => exact.is_eq(),
                    "<" => exact.is_lt(),
                    "<=" => exact.is_le(),
                    ">" => exact.is_gt(),
                    ">=" => exact.is_ge(),
                    _ => unreachable!(),
                };
                cases.push((a, b, expected));
            }
        }
        run_comparisons(&source, &cases);
    }
}

#[test]
fn signed_and_unsigned_register_domains_still_have_register_signatures() {
    for bounds in [
        "-9223372036854775808 to 9223372036854775807",
        "0 to 18446744073709551615",
    ] {
        let source = format!("identity(x in {bounds}) in {bounds}: return x\n");
        let output = emit(&source);
        assert!(output.status.success(), "{source}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("@identity(%arg0: i64) -> i64"),
            "{output:?}"
        );
    }
}

#[test]
fn an_internal_wide_selection_does_not_require_a_wide_calling_convention() {
    let source = "\
compare(a in -1 to 0, b in 0 to 18446744073709551615) in 0 to 1:
    selected: when b = 0 then a else b
    return when selected = b then 1 else 0
";
    run_comparisons(
        source,
        &[
            (-1, 0, false),
            (0, 0, true),
            (-1, u64::MAX, true),
            (0, u64::MAX, true),
        ],
    );
}

#[test]
fn an_array_element_that_needs_more_than_one_register_is_not_sized_as_eight_bytes() {
    let source = "\
identity(values [0 to 1208925819614629174706176; 2]) \
[0 to 1208925819614629174706176; 2]: return values
";
    let output = emit(source);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("func.func"),
        "{output:?}"
    );
}
