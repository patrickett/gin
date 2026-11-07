//! Lowering rules: Gin arithmetic to arm64, verified against the machine's semantics.
use machine_model::encode::Arm64;
use machine_model::instruction::{Instruction, Reg, X0, X1};
use machine_model::lower::{LowerError, Lowered, lower};
use machine_model::rules::{self, Rule};
use typecheck::{CheckedArithmetic, Limits, check_file};

fn compile(text: &str) -> Vec<(String, Result<Lowered, LowerError>)> {
    let file = parser::parse_strict(text).expect("strict lowering fixture");
    let checked = check_file(&file, Limits::default()).expect("lowering fixture checks");
    let convention = Arm64::load().unwrap().convention().unwrap();
    checked
        .functions
        .iter()
        .zip(&checked.bodies)
        .map(|(function, body)| {
            let body = body.as_ref().expect("body was recorded");
            (function.name.clone(), lower(&convention, function, body))
        })
        .collect()
}

fn only(text: &str) -> Vec<Instruction> {
    let mut all = compile(text);
    assert_eq!(all.len(), 1);
    all.remove(0).1.expect("lowers").instructions
}

fn add(dest: u8, left: u8, right: u8) -> Instruction {
    Instruction::Add {
        dest: Reg(dest),
        left: Reg(left),
        right: Reg(right),
    }
}

#[test]
fn every_rule_agrees_with_gin_arithmetic_and_refuses_what_wraps() {
    let samples = rules::samples();
    for rule in &rules::ARM64 {
        let verified = rule.verify(&samples).unwrap_or_else(|c| panic!("{c:?}"));
        assert!(
            verified.applicable > 100,
            "{:?}: {verified:?}",
            rule.operator
        );
        // Addition and subtraction of the extremes, and any large product, must wrap.
        assert!(verified.refused > 0, "{:?}: {verified:?}", rule.operator);
    }
}

#[test]
fn a_wrong_rule_is_caught_with_a_counterexample() {
    fn wrong(dest: Reg, left: Reg, right: Reg) -> Vec<Instruction> {
        vec![Instruction::Sub { dest, left, right }]
    }
    let rule = Rule {
        operator: CheckedArithmetic::Add,
        commutative: true,
        select: wrong,
    };
    let counterexample = rule.verify(&rules::samples()).expect_err("sub is not add");
    assert_eq!(counterexample.operator, CheckedArithmetic::Add);
    assert_ne!(
        i128::from(counterexample.left) + i128::from(counterexample.right),
        i128::from(counterexample.left) - i128::from(counterexample.right)
    );
}

#[test]
fn a_sum_of_parameters_is_one_add_in_place() {
    assert_eq!(
        only("sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b\n"),
        [add(0, 0, 1), Instruction::Ret]
    );
}

#[test]
fn known_operands_cost_a_constant_not_an_operation() {
    // `2 + 3` was evaluated when recorded, so the lowering sees the value 5.
    assert_eq!(
        only("five() in 0 to 16: return 2 + 3\n"),
        [Instruction::Movz { dest: X0, imm: 5 }, Instruction::Ret]
    );
}

#[test]
fn a_known_operand_beside_an_unknown_one_is_materialised_once() {
    assert_eq!(
        only("scaled(a in 0 to 10) in 0 to 30: return a * 3\n"),
        [
            Instruction::Movz { dest: X1, imm: 3 },
            Instruction::Mul {
                dest: X0,
                left: X0,
                right: X1
            },
            Instruction::Ret
        ]
    );
}

#[test]
fn a_constant_never_overwrites_a_parameter_that_is_not_read_yet() {
    // The constant is the first node, but `a` still lives in x0.
    let program = only("flipped(a in 0 to 10) in 0 to 30: return 3 * a\n");
    assert!(
        matches!(program[0], Instruction::Movz { dest, .. } if dest != X0),
        "{program:?}"
    );
}

#[test]
fn the_result_is_moved_to_x0_only_when_it_is_not_there() {
    assert_eq!(
        only("identity(x in 0 to 16) in 0 to 16: return x\n"),
        [Instruction::Ret]
    );
    assert_eq!(
        only("second(a in 0 to 16, b in 0 to 16) in 0 to 16: return b\n"),
        [Instruction::Mov { dest: X0, src: X1 }, Instruction::Ret]
    );
}

#[test]
fn a_repeated_operand_is_one_register() {
    assert_eq!(
        only("twice(x in 0 to 16) in 0 to 32: return x + x\n"),
        [add(0, 0, 0), Instruction::Ret]
    );
}

#[test]
fn nested_expressions_lower_and_are_proven_against_the_semantics() {
    let program =
        only("mixed(a in 0 to 10, b in 0 to 10) in -200 to 200: return (a + b) * (a - b)\n");
    assert_eq!(program.len(), 4, "{program:?}"); // add, sub, mul, ret: no moves, no spills
}

#[test]
fn what_cannot_be_lowered_fails_closed_for_a_stated_reason() {
    let error = |text: &str| compile(text).remove(0).1.expect_err("must not lower");
    assert!(matches!(
        error("wide(a in 0 to 9223372036854775807) in 0 to 18446744073709551614: return a + a\n"),
        LowerError::NotRepresentable { .. }
    ));
    assert!(matches!(
        error("big() in 0 to 100000: return 70000\n"),
        LowerError::ConstantOutOfRange(_)
    ));
    assert_eq!(
        error(
            "many(a in 0 to 1, b in 0 to 1, c in 0 to 1, d in 0 to 1, e in 0 to 1, \
             f in 0 to 1, g in 0 to 1, h in 0 to 1, i in 0 to 1) in 0 to 1: return a\n"
        ),
        LowerError::TooManyParameters(9, 8)
    );
    let calls = compile(
        "id(x in 0 to 16) in 0 to 16: return x\n\
         again(x in 0 to 16) in 0 to 16: return id(x)\n",
    );
    assert_eq!(
        calls[1].1.as_ref().unwrap_err(),
        &LowerError::Unsupported("a call")
    );
}

#[cfg(target_os = "macos")]
mod native {
    use std::process::Command;

    use machine_model::emit::functions;
    use machine_model::encode::Arm64;

    use super::*;

    const SOURCE: &str = "\
sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b
diff(a in 0 to 100, b in 0 to 100) in -100 to 100: return a - b
scaled(a in 0 to 10) in 0 to 30: return a * 3
flipped(a in 0 to 10) in 0 to 30: return 3 * a
mixed(a in 0 to 10, b in 0 to 10) in -200 to 200: return (a + b) * (a - b)
";

    const DRIVER: &str = r#"
#include <stdio.h>
extern long sum(long, long), diff(long, long), scaled(long), flipped(long), mixed(long, long);
int main(void) {
    long s = 0, d = 0, k = 0, f = 0, m = 0;
    for (long a = 0; a <= 10; a++) {
        k += scaled(a) * (1 + a);
        f += flipped(a) * (1 + a);
        for (long b = 0; b <= 10; b++) {
            long w = 1 + a * 11 + b;
            s += sum(a, b) * w;
            d += diff(a, b) * w;
            m += mixed(a, b) * w;
        }
    }
    printf("%ld %ld %ld %ld %ld\n", s, d, k, f, m);
    return 0;
}
"#;

    #[test]
    fn compiled_gin_runs_natively_and_agrees_with_exact_arithmetic() {
        let arm64 = Arm64::load().unwrap();
        let lowered: Vec<(String, Lowered)> = compile(SOURCE)
            .into_iter()
            .map(|(name, l)| (name, l.expect("lowers")))
            .collect();
        let listing: Vec<(&str, &[Instruction])> = lowered
            .iter()
            .map(|(n, l)| (n.as_str(), l.instructions.as_slice()))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let (assembly, driver, exe) = (
            dir.path().join("gin.s"),
            dir.path().join("driver.c"),
            dir.path().join("run"),
        );
        std::fs::write(&assembly, functions(&listing, &arm64).unwrap()).unwrap();
        std::fs::write(&driver, DRIVER).unwrap();
        let status = Command::new("cc")
            .arg("-o")
            .arg(&exe)
            .arg(&driver)
            .arg(&assembly)
            .status()
            .unwrap();
        assert!(status.success());
        let output = Command::new(&exe).output().unwrap();

        let mut expected = [0_i64; 5];
        for a in 0..=10_i64 {
            expected[2] += a * 3 * (1 + a);
            expected[3] += 3 * a * (1 + a);
            for b in 0..=10_i64 {
                let w = 1 + a * 11 + b;
                expected[0] += (a + b) * w;
                expected[1] += (a - b) * w;
                expected[4] += (a + b) * (a - b) * w;
            }
        }
        let expected: Vec<String> = expected.iter().map(i64::to_string).collect();
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            expected.join(" ")
        );
    }

    #[test]
    fn the_new_encodings_match_the_system_assembler() {
        let arm64 = Arm64::load().unwrap();
        let program = [
            Instruction::Add {
                dest: Reg(0),
                left: Reg(1),
                right: Reg(2),
            },
            Instruction::Sub {
                dest: Reg(3),
                left: Reg(4),
                right: Reg(5),
            },
            Instruction::Mul {
                dest: Reg(6),
                left: Reg(7),
                right: Reg(8),
            },
            Instruction::Mov {
                dest: Reg(0),
                src: Reg(9),
            },
            Instruction::Add {
                dest: Reg(15),
                left: Reg(15),
                right: Reg(30),
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        let (source, object) = (dir.path().join("r.s"), dir.path().join("r.o"));
        let text: String = program
            .iter()
            .map(|i| format!("    {}\n", i.mnemonic()))
            .collect();
        std::fs::write(&source, format!(".globl _f\n.p2align 2\n_f:\n{text}")).unwrap();
        assert!(
            Command::new("as")
                .args(["-arch", "arm64", "-o"])
                .arg(&object)
                .arg(&source)
                .status()
                .unwrap()
                .success()
        );
        let dump = Command::new("otool")
            .arg("-t")
            .arg(&object)
            .output()
            .unwrap();
        let words: Vec<u32> = String::from_utf8(dump.stdout)
            .unwrap()
            .lines()
            .skip(2)
            .flat_map(|line| {
                line.split_whitespace()
                    .skip(1)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .map(|word| u32::from_str_radix(&word, 16).unwrap())
            .collect();
        for (instruction, word) in program.iter().zip(words) {
            assert_eq!(
                arm64.encode(instruction).unwrap().word,
                word,
                "{instruction}"
            );
        }
    }
}

const LIBRARY: &str = include_str!("../gin/arm64.gin");

fn with_convention(replacements: &[(&str, &str)]) -> String {
    replacements
        .iter()
        .fold(LIBRARY.to_owned(), |text, (old, new)| {
            assert!(text.contains(old), "{old}");
            text.replace(old, new)
        })
}

#[test]
fn the_convention_is_read_from_gin() {
    let convention = Arm64::load().unwrap().convention().unwrap();
    assert_eq!(convention.arguments, (0..8).map(Reg).collect::<Vec<_>>());
    assert_eq!(convention.result, X0);
    assert_eq!(convention.allocatable, 16);
    assert_eq!(convention.constant_max, 65_535);
}

#[test]
fn editing_the_gin_convention_changes_the_lowering() {
    // Arguments in x1.., result in x3: a different ABI, stated only in Gin.
    let library = with_convention(&[
        (
            "argument_register(index in 0 to 7) in 0 to 30: return index",
            "argument_register(index in 0 to 7) in 0 to 30: return index + 1",
        ),
        (
            "result_register() in 0 to 30: return 0",
            "result_register() in 0 to 30: return 3",
        ),
    ]);
    let convention = Arm64::from_source(&library).unwrap().convention().unwrap();
    let text = "sum(a in 0 to 100, b in 0 to 100) in 0 to 200: return a + b\n";
    let file = parser::parse_strict(text).unwrap();
    let checked = check_file(&file, Limits::default()).unwrap();
    let lowered = lower(
        &convention,
        &checked.functions[0],
        checked.bodies[0].as_ref().unwrap(),
    )
    .unwrap();
    assert_eq!(
        lowered.instructions,
        [add(3, 1, 2), Instruction::Ret],
        "proven at x3, not x0"
    );
}

#[test]
fn a_convention_that_names_no_register_does_not_check() {
    let library = with_convention(&[(
        "result_register() in 0 to 30: return 0",
        "result_register() in 0 to 30: return 31",
    )]);
    assert!(Arm64::from_source(&library).is_err());
}

#[test]
fn a_convention_outside_the_allocatable_registers_is_refused() {
    let library = with_convention(&[(
        "allocatable() in 1 to 31: return 16",
        "allocatable() in 1 to 31: return 4",
    )]);
    assert!(Arm64::from_source(&library).unwrap().convention().is_err());
}
