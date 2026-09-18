use ast::ConstValue;
mod support;

use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::encoding::{EncodingArgument, EncodingItem, EncodingRegion, InstructionEncoder};
use typecheck::eval::{CompileTimeHost, Evaluator};
use typecheck::transform::{TransformCtx, transform};
use typecheck::{DefId, FileId, TypedFileAst};

const REGIONS: &str = include_str!("fixtures/instruction_regions.gin");

const SOURCE: &str = include_str!("fixtures/encoded_instructions.gin");

fn target() -> CompileTarget {
    CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap())
}

fn name(name: &str) -> Intern<String> {
    Intern::from_ref(name)
}
fn callable(value: &str) -> DefId {
    DefId(name(value))
}

fn transformed(source: &str) -> TypedFileAst {
    let parsed = TokenCursor::parse_source(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let mut ctx = TransformCtx::new();
    ctx.compile_target = target();
    transform(&parsed, FileId(0), &ctx)
}

fn checked(source: &str) -> TypedFileAst {
    let typed = transformed(source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    typed
}

fn instruction(encoder: &str, size: u32, args: Vec<EncodingArgument>) -> EncodingItem {
    EncodingItem::Instruction(InstructionEncoder::new(callable(encoder), size).with_arguments(args))
}

fn register(slot: &str) -> EncodingArgument {
    EncodingArgument::Register(name(slot))
}

#[test]
fn gin_encoders_evaluate_to_byte_sequences() {
    let typed = checked(SOURCE);
    let target = target();
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let value = evaluator
        .call(callable("KernelEncoder.encode"), &[])
        .unwrap();
    assert_eq!(
        value,
        ConstValue::FixedArray(vec![ConstValue::Int(15.into()), ConstValue::Int(5.into())].into())
    );
}

#[test]
fn gin_add_encoder_matches_llvm_for_every_register_pair() {
    use std::io::Write;
    use std::process::Command;
    use std::process::Stdio;

    let registers = [
        "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12",
        "r13", "r14", "r15",
    ];
    let mut source = String::from(".intel_syntax noprefix\n");
    for destination in registers {
        for source_register in registers {
            source.push_str(&format!("add {destination}, {source_register}\n"));
        }
    }
    let mut process = Command::new("llvm-mc")
        .args(["-triple=x86_64-unknown-linux-gnu", "--show-encoding"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("LLVM instruction encoder");
    process
        .stdin
        .take()
        .unwrap()
        .write_all(source.as_bytes())
        .unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = String::from_utf8(output.stdout).unwrap();
    let expected = output
        .lines()
        .filter_map(|line| {
            line.split_once("encoding: [").map(|(_, tail)| {
                tail.split(']')
                    .next()
                    .unwrap()
                    .split(',')
                    .map(|byte| {
                        u8::from_str_radix(byte.trim().trim_start_matches("0x"), 16).unwrap()
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 256);
    let typed = checked(SOURCE);
    let target = target();
    for (index, expected) in expected.iter().enumerate() {
        let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        let actual = evaluator
            .call(
                callable("AddEncoder.encode"),
                &[
                    ConstValue::Int(((index / 16) as u8).into()),
                    ConstValue::Int(((index % 16) as u8).into()),
                ],
            )
            .unwrap();
        let ConstValue::FixedArray(actual) = actual else {
            panic!("expected an encoded array")
        };
        let actual = actual
            .iter()
            .map(|value| u8::try_from(value.as_const_size_int().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            &actual,
            expected,
            "register pair {} {}",
            index / 16,
            index % 16
        );
    }
}

#[test]
fn rejects_non_byte_encoder_results_and_out_of_domain_arguments() {
    let typed = checked(&format!(
        "encode_invalid() Array(Int, 1): [256 as Int]\n{SOURCE}"
    ));
    let cases = [
        (
            instruction("encode_invalid", 1, vec![]),
            "type-encoding-byte",
        ),
        (
            instruction(
                "IncrementEncoder.encode",
                4,
                vec![EncodingArgument::Immediate(-1)],
            ),
            "type-encoding-argument",
        ),
    ];
    for (instruction, expected) in cases {
        let Err(error) = EncodingRegion::new(callable("branch_contract"), vec![instruction])
            .compile(typed.resolved_program(), &target())
        else {
            panic!("expected {expected}")
        };
        assert_eq!(error.code.slug(), expected, "{error:?}");
    }
}

#[test]
fn addition_encodes_the_compiler_selected_registers() {
    let typed = checked(SOURCE);
    let encoded = EncodingRegion::new(
        callable("add_contract"),
        vec![instruction(
            "AddEncoder.encode",
            3,
            vec![register("left"), register("right")],
        )],
    )
    .compile(typed.resolved_program(), &target())
    .unwrap();
    assert_eq!(encoded.register("left"), Some("rax"));
    assert_eq!(encoded.register("right"), Some("r9"));
    assert_eq!(encoded.bytes(), &[0x4c, 0x01, 0xc8]);
    let llvm = encoded.assembly().llvm_constraints().join(",");
    assert!(llvm.contains("={rax},0,{r9}"), "{llvm}");
}

#[test]
fn kernel_entry_preserves_register_ties_and_memory_effects() {
    let typed = checked(SOURCE);
    let encoded = EncodingRegion::new(
        callable("syscall_contract"),
        vec![instruction("KernelEncoder.encode", 2, vec![])],
    )
    .compile(typed.resolved_program(), &target())
    .unwrap();
    assert_eq!(encoded.bytes(), &[15, 5]);
    assert_eq!(
        encoded.assembly().memory.reads,
        [("buffer".into(), "length".into())]
    );
    let llvm = encoded.assembly().llvm_constraints().join(",");
    assert!(llvm.contains("={rax},0,{rdi},{rsi},{rdx}"), "{llvm}");
    assert!(
        llvm.contains("~{rcx}") && llvm.contains("~{r11}") && llvm.contains("~{memory}"),
        "{llvm}"
    );
}

#[test]
fn branch_encoder_receives_displacement_to_symbolic_destination() {
    let typed = checked(SOURCE);
    let encoded = EncodingRegion::new(
        callable("branch_contract"),
        vec![
            instruction(
                "JumpEncoder.encode",
                2,
                vec![EncodingArgument::Relative {
                    label: name("done"),
                    min: -128,
                    max: 127,
                }],
            ),
            instruction("IncrementEncoder.encode", 4, vec![register("value")]),
            EncodingItem::Label(name("done")),
            instruction("NopEncoder.encode", 1, vec![]),
        ],
    )
    .compile(typed.resolved_program(), &target())
    .unwrap();
    assert_eq!(encoded.bytes(), &[235, 4, 72, 131, 192, 1, 144]);
}

#[test]
fn backward_branch_uses_signed_displacement_before_byte_encoding() {
    let typed = checked(SOURCE);
    let encoded = EncodingRegion::new(
        callable("branch_contract"),
        vec![
            EncodingItem::Label(name("start")),
            instruction("NopEncoder.encode", 1, vec![]),
            instruction(
                "JumpEncoder.encode",
                2,
                vec![EncodingArgument::Relative {
                    label: name("start"),
                    min: -128,
                    max: 127,
                }],
            ),
        ],
    )
    .compile(typed.resolved_program(), &target())
    .unwrap();
    assert_eq!(encoded.bytes(), &[144, 235, 253]);
}

#[test]
fn rejects_unresolved_or_out_of_range_labels_and_incorrect_encoder_sizes() {
    let typed = checked(SOURCE);
    let cases = [
        (
            vec![instruction(
                "JumpEncoder.encode",
                2,
                vec![EncodingArgument::Relative {
                    label: name("missing"),
                    min: -128,
                    max: 127,
                }],
            )],
            "type-encoding-label",
        ),
        (
            vec![
                EncodingItem::Label(name("same")),
                EncodingItem::Label(name("same")),
            ],
            "type-encoding-label",
        ),
        (
            vec![instruction("NopEncoder.encode", 2, vec![])],
            "type-encoding-size",
        ),
        (
            vec![instruction(
                "IncrementEncoder.encode",
                4,
                vec![register("missing")],
            )],
            "type-encoding-operand",
        ),
        (
            vec![instruction("missing", 1, vec![])],
            "type-encoding-encoder",
        ),
        (
            vec![instruction("IncrementEncoder.encode", 4, vec![])],
            "type-encoding-encoder",
        ),
    ];
    for (items, expected) in cases {
        let result = EncodingRegion::new(callable("branch_contract"), items)
            .compile(typed.resolved_program(), &target());
        let Err(error) = result else {
            panic!("expected {expected}")
        };
        assert_eq!(error.code.slug(), expected, "{error:?}");
    }
    let mut items = vec![instruction(
        "JumpEncoder.encode",
        2,
        vec![EncodingArgument::Relative {
            label: name("far"),
            min: -128,
            max: 127,
        }],
    )];
    items.extend((0..128).map(|_| instruction("NopEncoder.encode", 1, vec![])));
    items.push(EncodingItem::Label(name("far")));
    let Err(error) = EncodingRegion::new(callable("branch_contract"), items)
        .compile(typed.resolved_program(), &target())
    else {
        panic!("expected displacement diagnostic")
    };
    assert_eq!(error.code.slug(), "type-encoding-displacement");
}

#[test]
fn rejects_invalid_contracts_and_unsatisfiable_register_constraints() {
    let cases = [
        (
            SOURCE.replace(r#"register: "rax", code: 0"#, r#"register: "rsp", code: 4"#),
            "type-encoding-register",
        ),
        (
            SOURCE.replace(r#"register: "rax", code: 0"#, r#"register: "rax", code: 1"#),
            "type-encoding-register",
        ),
        (
            SOURCE.replace(r#"outputs: ["left"]"#, r#"outputs: ["missing"]"#),
            "type-encoding-operand",
        ),
        (
            SOURCE.replace(r#"register: "rax", code: 0"#, r#"register: "r9", code: 9"#),
            "type-encoding-allocation",
        ),
        (
            SOURCE.replace(
                r#"target: #target.triple"#,
                r#"target: "aarch64-unknown-linux-gnu""#,
            ),
            "type-encoding-target",
        ),
    ];
    for (source, expected) in cases {
        let typed = checked(&source);
        let Err(error) = EncodingRegion::new(
            callable("add_contract"),
            vec![instruction(
                "AddEncoder.encode",
                3,
                vec![register("left"), register("right")],
            )],
        )
        .compile(typed.resolved_program(), &target()) else {
            panic!("expected {expected}")
        };
        assert_eq!(error.code.slug(), expected, "{error:?}");
    }
}

#[test]
fn gin_instruction_values_reach_checked_assembly() {
    let typed = checked(&format!("{REGIONS}\n{SOURCE}"));
    let cases = [
        ("add_words", ".byte 76, 1, 200"),
        ("kernel_entry", ".byte 15, 5"),
        ("skip_increment", ".byte 235, 4, 72, 131, 192, 1, 144"),
    ];
    for (function, expected) in cases {
        let typecheck::BindBody::Expr(expr) = typed.defs[&callable(function)].body else {
            panic!("expected expression body")
        };
        let typecheck::TypedExprKind::Asm {
            state: typecheck::assembly::AssemblyState::Checked(ref assembly),
            ..
        } = typed.exprs.kind[expr.as_usize()]
        else {
            panic!("expected checked instruction region")
        };
        assert_eq!(assembly.template, expected);
    }
}

#[test]
fn instruction_protocol_uses_shapes_and_encoder_identity() {
    let source = format!("{REGIONS}\n{SOURCE}")
        .replace("AddEncoder", "SumBytes")
        .replace("Instruction", "Operation")
        .replace("Region", "Program")
        .replace("Argument", "Operand")
        .replace("Register", "Location");
    let typed = checked(&source);
    let typecheck::BindBody::Expr(expr) = typed.defs[&callable("add_words")].body else {
        panic!("expression body")
    };
    let typecheck::TypedExprKind::Asm {
        state: typecheck::assembly::AssemblyState::Checked(ref assembly),
        ..
    } = typed.exprs.kind[expr.as_usize()]
    else {
        panic!("checked region")
    };
    assert_eq!(assembly.template, ".byte 76, 1, 200");
}

#[test]
fn source_instruction_contracts_reject_invalid_encoders_operands_and_memory() {
    let original = format!("{REGIONS}\n{SOURCE}");
    let cases = [
        (
            "encoder: AddEncoder",
            "encoder: NopEncoder",
            "type-encoding-encoder",
        ),
        (
            "AddEncoder has\n    encode(",
            "AddEncoder has\n    unrelated(",
            "type-encoding-encoder",
        ),
        ("size: 3", "size: 4", "type-encoding-size"),
        ("size: 3", "size: 0", "type-encoding-size"),
        (
            r#"Register(register: "right")"#,
            r#"Register(register: "missing")"#,
            "type-encoding-operand",
        ),
        (
            r#"Relative(relative: "done", min: -128, max: 127)"#,
            r#"Relative(relative: "missing", min: -128, max: 127)"#,
            "type-encoding-label",
        ),
        (
            r#"Relative(relative: "done", min: -128, max: 127)"#,
            r#"Relative(relative: "done", min: -128, max: 3)"#,
            "type-encoding-displacement",
        ),
        ("left: a, right: b", "left: a", "type-invalid-assembly"),
        ("buffer @Int", "buffer Int", "type-invalid-assembly"),
    ];
    for (before, after, code) in cases {
        assert!(original.contains(before));
        let typed = transformed(&original.replace(before, after));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == code),
            "{before}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn source_branch_resolves_a_backward_symbolic_destination() {
    let source = format!("{REGIONS}\n{SOURCE}");
    let start = source
        .find("        items: (instruction0: Instruction(encoder: JumpEncoder")
        .unwrap();
    let end = source[start..].find('\n').unwrap() + start;
    let source = format!(
        "{}{}{}",
        &source[..start],
        "        items: (item0: Mark(label: \"start\"), item1: Instruction(encoder: NopEncoder, size: 1, arguments: ()), item2: Instruction(encoder: JumpEncoder, size: 2, arguments: (argument0: Relative(relative: \"start\", min: -128, max: 127))))",
        &source[end..]
    );
    let typed = checked(&source);
    let typecheck::BindBody::Expr(expr) = typed.defs[&callable("skip_increment")].body else {
        panic!("expression body")
    };
    let typecheck::TypedExprKind::Asm {
        state: typecheck::assembly::AssemblyState::Checked(ref assembly),
        ..
    } = typed.exprs.kind[expr.as_usize()]
    else {
        panic!("checked region")
    };
    assert_eq!(assembly.template, ".byte 144, 235, 253");
}

#[test]
fn structured_instructions_lower_through_the_normal_backend() {
    let (mlir, diagnostics) =
        support::codegen_assembly_to_mlir_text(&format!("{REGIONS}\n{SOURCE}"), "instructions.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(mlir.contains("llvm.inline_asm"), "{mlir}");
    assert!(mlir.contains("={rax},0,{r9}"), "{mlir}");
    assert!(mlir.contains("~{rcx},~{r11},~{cc},~{memory}"), "{mlir}");
}

#[test]
fn gin_instruction_values_emit_disassemblable_objects() {
    let source = format!("{REGIONS}\n{SOURCE}");
    checked(&source);
    let path = std::env::temp_dir().join(format!("gin-source-encoding-{}.o", std::process::id()));
    let (ok, diagnostics) =
        support::codegen_assembly_to_object(&source, "instructions.gin", target(), &path);
    assert!(ok && diagnostics.is_empty(), "{diagnostics:?}");
    let output = std::process::Command::new("llvm-objdump")
        .args(["-d", "--x86-asm-syntax=intel"])
        .arg(&path)
        .output()
        .unwrap();
    let _ = std::fs::remove_file(path);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let assembly = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "add_words",
        "add\trax, r9",
        "kernel_entry",
        "syscall",
        "skip_increment",
        "jmp",
    ] {
        assert!(
            assembly.contains(expected),
            "missing {expected}: {assembly}"
        );
    }
}
