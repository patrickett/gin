//! Hello world on macOS arm64 through `libSystem`, as instructions-as-data:
//! `int main(void) { write(1, "hello world\n", 12); return 0; }`
use std::collections::BTreeMap;

use machine_model::abstract_machine::Effects;
use machine_model::encode::{Arm64, EncodeError};
use machine_model::instruction::{Instruction, Reg, SP, X0, X1, X2, X29, X30};
use machine_model::{Data, check, run};

const RESULT: (u64, u64) = (0, 255);

fn hello() -> Vec<Instruction> {
    vec![
        Instruction::Stp {
            first: X29,
            second: X30,
            base: SP,
            offset: -16,
        },
        Instruction::movz(X0, 1),
        Instruction::adrp_page(X1, "msg"),
        Instruction::add_low12(X1, X1, "msg"),
        Instruction::movz(X2, 12),
        Instruction::bl("_write"),
        Instruction::movz(X0, 0),
        Instruction::Ldp {
            first: X29,
            second: X30,
            base: SP,
            offset: 16,
        },
        Instruction::Ret,
    ]
}

fn data() -> Data {
    BTreeMap::from([("msg".to_owned(), b"hello world\n".to_vec())])
}

fn registers(list: &[Reg]) -> std::collections::BTreeSet<Reg> {
    list.iter().copied().collect()
}

#[test]
fn the_program_checks_and_its_effects_are_derived_from_the_semantics() {
    let effects = check(&hello(), &data(), RESULT).expect("hello world checks");
    // `Bl(_write)` reads its three arguments, writes the count and the link register,
    // and destroys the caller-saved registers that held something. Nobody declared this.
    assert_eq!(
        effects[5],
        Effects {
            reads: registers(&[X0, X1, X2]),
            writes: registers(&[X0, X30]),
            clobbers: registers(&[X1, X2]),
        }
    );
    assert_eq!(effects[0].reads, registers(&[X29, X30, SP]));
    assert_eq!(effects[0].writes, registers(&[SP]));
}

#[test]
fn emulation_prints_hello_world_and_exits_zero() {
    let (stdout, status) = run(&hello(), &data()).unwrap();
    assert_eq!(stdout, b"hello world\n");
    assert_eq!(status, 0);
}

#[test]
fn bad_programs_are_rejected_for_the_stated_reason() {
    let without = |index: usize| {
        let mut program = hello();
        program.remove(index);
        program
    };
    let replace = |index: usize, instruction: Instruction| {
        let mut program = hello();
        program[index] = instruction;
        program
    };
    let mut after_call = hello();
    after_call.insert(6, Instruction::add_low12(Reg(3), X1, "msg"));
    let cases = [
        ("drop Movz(x2, 12)", without(4), "read of unset x2"),
        (
            "length 13",
            replace(4, Instruction::movz(X2, 13)),
            "reads past msg (12 bytes)",
        ),
        ("drop Movz(x0, 0)", without(6), "x0 is not proven"),
        ("drop Ldp", without(7), "callee-saved"),
        ("read x1 after the call", after_call, "read of unset x1"),
        (
            "AddLow12 without the page",
            replace(2, Instruction::movz(X1, 0)),
            "needs the page of msg",
        ),
        (
            "fd 2",
            replace(1, Instruction::movz(X0, 2)),
            "not proven to be 1",
        ),
    ];
    for (name, program, expected) in cases {
        let error = check(&program, &data(), RESULT)
            .expect_err(name)
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn words_come_from_evaluating_the_gin_library() {
    let arm64 = Arm64::load().expect("the Gin library checks");
    let word = |instruction: &Instruction| arm64.encode(instruction).unwrap().word;
    let program = hello();
    let words: Vec<u32> = program.iter().map(word).collect();
    assert_eq!(
        words,
        [
            0xA9BF7BFD, 0xD2800020, 0x90000001, 0x91000021, 0xD2800182, 0x94000000, 0xD2800000,
            0xA8C17BFD, 0xD65F03C0
        ]
    );
}

#[test]
fn a_field_outside_its_gin_domain_is_no_instruction() {
    let arm64 = Arm64::load().unwrap();
    let out_of_domain = |instruction: Instruction| {
        matches!(
            arm64.encode(&instruction),
            Err(EncodeError::OutOfDomain { .. })
        )
    };
    assert!(out_of_domain(Instruction::movz(X0, 70_000)));
    assert!(out_of_domain(Instruction::movz(SP, 1)));
    assert!(matches!(
        arm64.encode(&Instruction::Stp {
            first: X29,
            second: X30,
            base: SP,
            offset: 4
        }),
        Err(EncodeError::PairOffset(4))
    ));
}

#[test]
fn another_gin_module_folds_an_encoding_at_compile_time() {
    use i256::I256;
    use typecheck::{CheckedSourceBodyKind, Limits, check_module};
    let arm64 = Arm64::load().unwrap();
    let file = parser::parse_strict(
        "use arm64 movz\nfirst_word() in 0 to 4294967295: return movz(0, 1)\n",
    )
    .unwrap();
    let app = check_module("app", &file, &[arm64.module()], Limits::default()).unwrap();
    let body = app.checked().bodies[0].as_ref().unwrap();
    let CheckedSourceBodyKind::Block { result, .. } = body.expressions[body.root as usize].kind
    else {
        panic!("root is a block");
    };
    let CheckedSourceBodyKind::Return { value } = body.expressions[result as usize].kind else {
        panic!("block ends in a return");
    };
    // `mov x0, #1` is the constant 0xD2800020: no call to `movz` survives.
    assert_eq!(
        body.expressions[value as usize].kind,
        CheckedSourceBodyKind::Integer(I256::from(0xD2800020_i64))
    );
}

#[cfg(target_os = "macos")]
mod native {
    use std::path::Path;
    use std::process::Command;

    use machine_model::emit::assembly;
    use machine_model::encode::RelocKind;

    use super::*;

    fn tool(program: &str, arguments: &[&Path]) -> String {
        let output = Command::new(program)
            .args(arguments)
            .output()
            .expect(program);
        assert!(output.status.success(), "{program}: {output:?}");
        String::from_utf8(output.stdout).unwrap()
    }

    #[test]
    fn every_encoding_and_relocation_matches_the_system_assembler() {
        let arm64 = Arm64::load().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (source, object) = (
            dir.path().join("reference.s"),
            dir.path().join("reference.o"),
        );
        let mut text = String::from(".globl _main\n.p2align 2\n_main:\n");
        for instruction in hello() {
            text += &format!("    {}\n", instruction.mnemonic());
        }
        text += ".section __TEXT,__const\nmsg:\n    .byte 0\n";
        std::fs::write(&source, text).unwrap();
        let status = Command::new("as")
            .args(["-arch", "arm64", "-o"])
            .arg(&object)
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());

        let dump = tool("otool", &[Path::new("-t"), &object]);
        let words: Vec<u32> = dump
            .lines()
            .skip(2)
            .flat_map(|line| line.split_whitespace().skip(1))
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect();
        let relocations = tool("otool", &[Path::new("-rv"), &object]);
        let found: BTreeMap<usize, (RelocKind, String)> = relocations
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let kind = match *fields.get(4)? {
                    "BR26" => RelocKind::Branch26,
                    "PAGE21" => RelocKind::Page21,
                    "PAGOF12" => RelocKind::PageOff12,
                    _ => return None,
                };
                let address = usize::from_str_radix(fields[0], 16).ok()?;
                Some((address / 4, (kind, fields.get(6)?.to_string())))
            })
            .collect();

        for (index, instruction) in hello().iter().enumerate() {
            let encoded = arm64.encode(instruction).unwrap();
            assert_eq!(words[index], encoded.word, "word of {instruction}");
            let expected = encoded.reloc.map(|reloc| (reloc.kind, reloc.symbol));
            assert_eq!(
                found.get(&index).cloned(),
                expected,
                "relocation of {instruction}"
            );
        }
    }

    #[test]
    fn the_linked_executable_prints_what_the_model_printed() {
        let arm64 = Arm64::load().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (source, exe) = (dir.path().join("hello.s"), dir.path().join("hello"));
        std::fs::write(&source, assembly(&hello(), &data(), &arm64).unwrap()).unwrap();
        let status = Command::new("cc")
            .arg("-o")
            .arg(&exe)
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        let real = Command::new(&exe).output().unwrap();
        let (stdout, status) = run(&hello(), &data()).unwrap();
        assert_eq!(real.stdout, stdout);
        assert_eq!(u64::from(real.status.code().unwrap() as u32), status);
    }
}
