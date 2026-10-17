use flask::{CompileTarget, TargetTriple};
use parser::parse_from_str;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    check_target(source, "x86_64-unknown-linux-gnu")
}

fn check_target(source: &str, target: &str) -> TypedFileAst {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
    transform(&parsed, FileId(0), &context)
}

const SOURCE: &str = r#"
Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return
Inputs has
    value Word
    finish(self):
        return
#phantom(a, b)
Signature(a, b) has
    version Word
    finish(self):
        return
sig Signature(Inputs, Word): Signature(version: 1)
accumulator := [architecture: "x86_64", bank: "general", index: 0, width: 64]
code := [
    signature: sig
    target: "x86_64-unknown-linux-gnu"
    bytes: [72, 255, 192]
    inputs: [value: accumulator]
    outputs: [value: accumulator]
    clobbers: []
    reads: []
    writes: []
]
increment(value Word) Word: return #emit(code, Inputs(value: value))
"#;

#[test]
fn required_code_construction_preserves_traps_and_live_frames() {
    let source = SOURCE.replace(
        "#emit(code, Inputs(value: value))",
        "#emit(build(1), Inputs(value: value))",
    ) + "\nbuild(flag Word):\n    retained Word: 7\n    while flag is 1 do\n        iteration: flag\n    return code\n";
    let typed = check(&source);
    let condition = typed
        .eval_conditions
        .iter()
        .find(|condition| condition.failure.diagnostic.code.slug() == "compile-time-fuel-exhausted")
        .expect("required compile-time evaluation retains the original trap");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| **diagnostic == condition.failure.diagnostic)
    );
    let frame = condition
        .failure
        .frames
        .iter()
        .find(|frame| {
            frame
                .function
                .is_some_and(|function| function.0.as_str() == "build")
        })
        .unwrap();
    assert_eq!(
        frame
            .locals
            .iter()
            .find(|local| local.name.as_str() == "retained")
            .unwrap()
            .value,
        Ok(ast::ConstValue::Int(7.into()))
    );
    assert!(condition.failure.frames.last().unwrap().function.is_none());
    assert!(!condition.failure.diagnostic.span.is_empty());
}

#[test]
fn library_emission_accepts_compile_time_local_specifications() {
    let source = SOURCE.replace(
        "increment(value Word) Word: return #emit(code, Inputs(value: value))",
        "increment(value Word) Word:\n    local: code\n    return #emit(local, Inputs(value: value))",
    );
    let typed = transform(&parse_from_str(&source), FileId(0), &TransformCtx::new());
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn library_emission_rejects_runtime_local_specifications() {
    let source = SOURCE.replace(
        "increment(value Word) Word: return #emit(code, Inputs(value: value))",
        "increment(value Word) Word:\n    local: [signature: sig, target: #target.triple, bytes: [value], inputs: [value: accumulator], outputs: [value: accumulator], clobbers: [], reads: [], writes: []]\n    return #emit(local, Inputs(value: value))",
    );
    let typed = transform(&parse_from_str(&source), FileId(0), &TransformCtx::new());
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-cannot-call-comptime-with-runtime-args"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn expression_uses_typed_signature_and_register_values() {
    let typed = check(SOURCE);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let assembly = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            typecheck::TypedExprKind::Emit {
                state: typecheck::assembly::AssemblyState::Checked(assembly),
                ..
            } => Some(assembly),
            _ => None,
        })
        .expect("checked emission");
    assert_eq!(assembly.template, ".byte 72, 255, 192");
    assert_eq!(assembly.result.as_deref(), Some("rax"));
    assert!(assembly.inputs[0].tie);
}

#[test]
fn library_methods_build_and_patch_instruction_storage() {
    let typed = check(&test_fixtures::emission::buffered_increment_source(
        "x86_64-unknown-linux-gnu",
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        typecheck::TypedExprKind::Emit { state: typecheck::assembly::AssemblyState::Checked(code), .. }
        if code.template == ".byte 72, 255, 192"
    )));
}

#[test]
fn rejects_instruction_buffer_length_outside_storage() {
    let source = SOURCE.replace(
        "bytes: [72, 255, 192]",
        "bytes: [storage: [72, 255, 192], length: 4]",
    );
    let typed = check(&source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("capacity")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn rejects_missing_record_outputs() {
    let source = SOURCE
        .replace("Signature(Inputs, Word)", "Signature(Inputs, Outputs)")
        .replace(
            "increment(value Word) Word:",
            "increment(value Word) Outputs:",
        )
        .replace("outputs: [value: accumulator]", "outputs: []")
        + "\nOutputs has result Word\n";
    let typed = check(&source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("missing assembly output")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn code_may_have_no_runtime_inputs() {
    let source = SOURCE
        .replace("Signature(Inputs, Word)", "Signature([], Word)")
        .replace("inputs: [value: accumulator]", "inputs: []")
        .replace("Inputs(value: value)", "[]");
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

const MEMORY_SOURCE: &str = r#"
Word is in 0 to 18446744073709551615
Word has
    finish(self):
        return
Byte is in 0 to 255
Byte has
    finish(self):
        return
Arguments has
    pointer @Byte, count Word
    finish(self):
        return
#phantom(input, output)
Contract(input, output) has
    version Byte
    finish(self):
        return
#phantom(input, value)
Operand(input, value) has
    index Word
    finish(self):
        return
sig Contract(Arguments, ()): Contract(version: 1)
pointer Operand(Arguments, @Byte): Operand(index: 0)
count Operand(Arguments, Word): Operand(index: 1)
code := [signature: sig, target: #target.triple, bytes: [144]
    inputs: [count: [architecture: "x86_64", bank: "general", index: 6, width: 64], pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64]]
    outputs: [], clobbers: [], reads: [[pointer: pointer, length: count]], writes: []]
read(pointer @Byte, count Word): return #emit(code, Arguments(pointer: pointer, count: count))
"#;

#[test]
fn typed_memory_handles_follow_signature_order() {
    let typed = check(MEMORY_SOURCE);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        typecheck::TypedExprKind::Emit { state: typecheck::assembly::AssemblyState::Checked(code), .. }
        if code.memory.reads == vec![("pointer".into(), "count".into())]
    )));
}

#[test]
fn checked_emission_only_invalidates_memory_when_its_contract_writes() {
    for (writes, invalidates) in [("[]", false), ("[[pointer: pointer, length: count]]", true)] {
        let source = MEMORY_SOURCE.replace("writes: []", &format!("writes: {writes}"))
            + "\nAddress is @Byte\nAddress has finish(self): return\npositive(value ref Byte and value > 0) Byte: return 0\ncheck(value Byte and value > 0) Byte:\n    pointer Address: @value\n    read(pointer, 1)\n    return positive(value)\n";
        let typed = check(&source);
        let effects =
            &typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("read"))].effects;
        assert_eq!(effects.unknown_memory, invalidates);
        if invalidates {
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-parameter-refinement-unproven"),
                "{:?}",
                typed.all_flaws()
            );
        } else {
            assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        }
    }
}

#[test]
fn generic_library_records_compose_memory_effects() {
    let source = MEMORY_SOURCE.replace("reads: [[pointer: pointer, length: count]]", "reads: [access]")
        .replace("code :=", "Access(i, p, l) has\n    pointer Operand(i, p), length Operand(i, l)\n    finish(self):\n        return\naccess Access(Arguments, @Byte, Word): Access(pointer: pointer, length: count)\ncode :=");
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn rejects_operand_handle_with_wrong_index_or_signature() {
    for source in [
        MEMORY_SOURCE.replace("Operand(index: 0)", "Operand(index: 1)"),
        MEMORY_SOURCE.replace(
            "pointer Operand(Arguments, @Byte)",
            "pointer Operand(Word, @Byte)",
        ),
    ] {
        let typed = check(&source);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains("operand handle type")),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn qualified_library_code_keeps_emission_dependencies() {
    let source = test_fixtures::emission::increment_source("x86_64-unknown-linux-gnu");
    let parsed = parse_from_str(&source).qualify_module_defs("circuit");
    assert!(
        parsed.defs.keys().any(|id| id.0.as_str() == "circuit.code"),
        "{:#?}",
        parsed.defs.keys().collect::<Vec<_>>()
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&parsed, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn descriptor_names_do_not_select_registers() {
    let source = SOURCE.replace("accumulator", "x9");
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn unrelated_function_is_not_a_register() {
    let source = SOURCE.replace("inputs: [value: accumulator]", "inputs: [value: x9()]")
        + "\nx9() Word: return 9\n";
    let typed = check(&source);
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn rejects_wrong_register_architecture() {
    let typed = check(&SOURCE.replace("architecture: \"x86_64\"", "architecture: \"aarch64\""));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("architecture")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn rejects_malformed_code_contracts() {
    for (before, after, message) in [
        ("version: 1", "version: 2", "version"),
        ("bytes: [72, 255, 192]", "bytes: [256]", "bytes"),
        ("bytes: [72, 255, 192]", "bytes: []", "nonempty"),
        ("width: 64", "width: 63", "width"),
        ("clobbers: []", "clobbers: [accumulator]", "overlaps"),
        (
            "writes: []",
            "writes: [[pointer: 0, length: 3]]",
            "unknown input",
        ),
        (
            "outputs: [value: accumulator]",
            "outputs: []",
            "exactly one output",
        ),
        (
            "outputs: [value: accumulator]",
            "outputs: [other: accumulator]",
            "named `value`",
        ),
    ] {
        let typed = check(&SOURCE.replace(before, after));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, diagnostic)| diagnostic.message.contains(message)),
            "{after}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn rejects_same_width_but_different_input_type() {
    let source = SOURCE.replace("Inputs(value: value))", "Other(value: value))")
        + "\nOther has value Word\n";
    let typed = check(&source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("input type")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn result_type_is_inferred_from_signature_inside_a_body() {
    let source = SOURCE.replace("increment(value Word) Word: return #emit(code, Inputs(value: value))", "increment(value Word) Word:\n    result := #emit(code, Inputs(value: value))\n    return result");
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn local_code_description_is_compile_time_data() {
    let start = SOURCE.find("code :=").unwrap();
    let end = SOURCE.find("increment(").unwrap();
    let local = SOURCE[start..end]
        .lines()
        .map(|line| format!("    {line}\n"))
        .collect::<String>();
    let source = format!(
        "{}increment(value Word) Word:\n{local}    return #emit(code, Inputs(value: value))\n",
        &SOURCE[..start]
    );
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

fn local_code(prefix: &str, bytes: &str) -> String {
    let start = SOURCE.find("code :=").unwrap();
    let end = SOURCE.find("increment(").unwrap();
    let code = SOURCE[start..end]
        .replace("[72, 255, 192]", bytes)
        .lines()
        .map(|line| format!("    {line}\n"))
        .collect::<String>();
    format!(
        "{}increment(value Word) Word:\n{prefix}{code}    return #emit(code, Inputs(value: value))\n",
        &SOURCE[..start]
    )
}

#[test]
fn code_construction_executes_ordinary_mutating_helpers() {
    let source = local_code(
        "    buffer Bytes(Byte, 3): [72 as Byte, 255 as Byte, 0 as Byte]\n    patch(buffer)\n",
        "buffer",
    ) + &format!(
        "\n{}replace(previous Byte, next Byte) Byte: return next\npatch(bytes mut Bytes(Byte, 3)):\n    bytes.[2]:: replace(bytes.[2], 192 as Byte)\n    return\n\n#fixed_array(element, count)\nBytes(element, count BigInt) has\n    finish(self):\n        return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let typed = check(&source);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}\n{:?}",
        typed.all_flaws()
    );
    let assembly = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            typecheck::TypedExprKind::Emit {
                state: typecheck::assembly::AssemblyState::Checked(assembly),
                ..
            } => Some(assembly),
            _ => None,
        })
        .unwrap();
    assert_eq!(assembly.template, ".byte 72, 255, 192");
    assert_eq!(assembly.construction.len(), 3);
}

#[test]
fn runtime_values_cannot_supply_instruction_bytes() {
    let typed = check(&local_code("", "[value]"));
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn construction_state_cannot_escape_into_runtime_arguments() {
    let source = local_code(
        "    buffer Bytes(Byte, 3): [72 as Byte, 255 as Byte, 192 as Byte]\n",
        "buffer",
    )
    .replace("Inputs(value: value)", "Inputs(value: buffer.[0] as Word)")
        + &format!(
            "\n{}#fixed_array(element, count)\nBytes(element, count BigInt) has\n    finish(self):\n        return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
    let typed = check(&source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("share mutable local state")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn rejects_overlapping_register_aliases() {
    let source = SOURCE.replace(
        "inputs: [value: accumulator]",
        "inputs: [value: accumulator, extra: accumulator]",
    );
    let typed = check(&source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains("overlapping register")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn code_can_be_reused_by_multiple_runtime_emissions() {
    let source = local_code("", "[72, 255, 192]").replace(
        "    return #emit(code, Inputs(value: value))",
        "    first := #emit(code, Inputs(value: value))\n    return #emit(code, Inputs(value: first))",
    );
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn rejects_pointer_operands_in_narrow_registers() {
    let source = MEMORY_SOURCE
        .replace("x86_64", "aarch64")
        .replace("bytes: [144]", "bytes: [31, 32, 3, 213]")
        .replace("index: 7, width: 64", "index: 7, width: 32");
    let typed = check_target(&source, "aarch64-unknown-linux-gnu");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("pointer width")),
        "{:?}",
        typed.all_flaws()
    );
}
