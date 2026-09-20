mod support;

use ast::BinOp;
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::cursor::TokenCursor;
use std::process::Command;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn source(arm: bool) -> String {
    let (left, right, size, bytes) = if arm {
        (
            "x0",
            "x9",
            4,
            "((destination | ((destination & 7) << 5)) as Byte), ((destination >> 3) as Byte), (source as Byte), (139 as Byte)",
        )
    } else {
        (
            "rax",
            "r9",
            3,
            "((72 | ((source >> 3) << 2) | (destination >> 3)) as Byte), (1 as Byte), ((192 | ((source & 7) << 3) | (destination & 7)) as Byte)",
        )
    };
    format!(
        r#"Word is in -9223372036854775808 to 9223372036854775807
Byte is in 0 to 255
Count is in 0 to 2147483647
Choice(slot_type Type, register_type Type) has slot slot_type, register register_type, code Count
Operand(register_type Type) has register register_type
Access has pointer Count, length Count
Instruction(encoder_type Type, arguments_type Type) has encoder encoder_type, size Count, arguments arguments_type
Region(contract_type Type, items_type Type) has contract contract_type, items items_type
Encoder has
    encode(destination is in 0 to 255, source is in 0 to 255) Sequence(Byte, {size}): [{bytes}]
Contract(target_type Type, choices_type Type, inputs_type Type, outputs_type Type, clobbers_type Type, reads_type Type, writes_type Type) has
    target target_type
    choices choices_type
    inputs inputs_type
    outputs outputs_type
    clobbers clobbers_type
    reads reads_type
    writes writes_type
    evaluate(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) a Word: (a + b) as Word

word_sum_result(a Word, b Word) a Word: #asm(region(), a: a, b: b)
#infix('+')
word_sum(a Word, b Word) Word: word_sum_result(a, b).a
calculate(a Word, b Word) Word: a + b

contract():
return Contract(
        target: #target.triple
        choices: [choice0: Choice(slot: 'a', register: '{left}', code: 0), choice1: Choice(slot: 'b', register: '{right}', code: 9)]
        inputs: [input0: 'a', input1: 'b']
        outputs: ['a']
        clobbers: ['cc']
        reads: []
        writes: []
    )

region():
return Region(
        contract: contract()
        items: [instruction0: Instruction(encoder: Encoder, size: {size}, arguments: [argument0: Operand(register: 'a'), argument1: Operand(register: 'b')])]
    )


#fixed_array(element, count)
Sequence(element Type, count BigInt) has
"#
    )
}

#[test]
fn instruction_evaluation_requires_a_compatible_gin_method() {
    let declaration = "    evaluate(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) a Word: (a + b) as Word\n";
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let mut ctx = TransformCtx::new();
    ctx.compile_target = target.clone();
    let without_method = source(false).replace(declaration, "");
    let typed = transform(&TokenCursor::parse_source(&without_method), FileId(0), &ctx);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let mut evaluator = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    );
    assert_eq!(
        evaluator
            .call(
                DefId(Intern::from_ref("calculate")),
                &[ast::ConstValue::ONE, ast::ConstValue::ONE]
            )
            .unwrap_err()
            .code
            .slug(),
        "compile-time-runtime-dependency"
    );
    for (replacement, expected) in [
        ("    evaluate(a Word) Word: a\n", "requires 2 parameters"),
        (
            "    evaluate(a Word, b Word) Count: 0\n",
            "must return the assembly result type",
        ),
        (
            "    evaluate(a Count, b Count) Word: 0\n",
            "must match its operand type",
        ),
    ] {
        let source = source(false).replace(declaration, replacement);
        let parsed = TokenCursor::parse_source(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, diagnostic)| diagnostic.message.contains(expected)),
            "{replacement}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn library_operator_dispatches_to_encoded_addition_on_both_targets() {
    for (triple, arm, expected) in [
        ("x86_64-unknown-linux-gnu", false, "add\trax, r9"),
        ("aarch64-unknown-linux-gnu", true, "add\tx0, x0, x9"),
    ] {
        let source = source(arm);
        let target = CompileTarget::Concrete(TargetTriple::parse(triple).unwrap());
        let mut ctx = TransformCtx::new();
        ctx.compile_target = target.clone();
        let parsed = TokenCursor::parse_source(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let mut evaluator = typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        );
        for (a, b) in [
            (20_i64, 22_i64),
            (-20, -22),
            (-1, 1),
            (i64::MAX, 1),
            (i64::MIN, -1),
            (i64::MIN, i64::MAX),
            (0, 0),
        ] {
            let values = [
                ast::ConstValue::Int(a.into()),
                ast::ConstValue::Int(b.into()),
            ];
            assert_eq!(
                evaluator.call(DefId(Intern::from_ref("calculate")), &values),
                Ok(ast::ConstValue::Int(a.wrapping_add(b).into())),
                "{triple}: {a} + {b}"
            );
        }
        let BindBody::Expr(expr) = typed.defs[&DefId(Intern::from_ref("calculate"))].body else {
            panic!("operator caller body");
        };
        assert!(
            matches!(typed.exprs.kind_of(expr), Some(TypedExprKind::FnCall { target, .. }) if target.0.as_str() == "word_sum"),
            "{:?}",
            typed.exprs.kind_of(expr)
        );
        assert!(
            !typed
                .exprs
                .kind
                .iter()
                .any(|kind| matches!(kind, TypedExprKind::Binary { op: BinOp::Add, .. }))
        );
        let path =
            std::env::temp_dir().join(format!("gin-operator-{}-{arm}.o", std::process::id()));
        let (ok, diagnostics) =
            support::codegen_assembly_to_object(&source, "operator.gin", target, &path);
        assert!(ok && diagnostics.is_empty(), "{diagnostics:?}");
        let output = Command::new("llvm-objdump")
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
        let disassembly = String::from_utf8(output.stdout).unwrap();
        assert!(disassembly.contains(expected), "{disassembly}");
    }
}

#[test]
#[cfg(all(
    any(target_arch = "aarch64", target_arch = "x86_64"),
    any(target_os = "macos", target_os = "linux")
))]
fn encoded_library_addition_executes_with_signed_word_wrapping() {
    let arm = cfg!(target_arch = "aarch64");
    let target = CompileTarget::Concrete(
        TargetTriple::parse(match (arm, cfg!(target_os = "macos")) {
            (true, true) => "aarch64-apple-darwin",
            (false, true) => "x86_64-apple-darwin",
            (true, false) => "aarch64-unknown-linux-gnu",
            (false, false) => "x86_64-unknown-linux-gnu",
        })
        .unwrap(),
    );
    let directory =
        std::env::temp_dir().join(format!("gin-operator-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let object = directory.join("operator.o");
    let (ok, diagnostics) =
        support::codegen_assembly_to_object(&source(arm), "operator.gin", target, &object);
    assert!(ok && diagnostics.is_empty(), "{diagnostics:?}");
    let driver = directory.join("main.c");
    std::fs::write(
        &driver,
        r#"#include <stdint.h>
#include <limits.h>
extern int64_t calculate(int64_t, int64_t);
int main(void) {
    const int64_t cases[][3] = {
        {20, 22, 42}, {-20, -22, -42}, {-20, 22, 2},
        {INT64_MAX, 1, INT64_MIN}, {INT64_MIN, -1, INT64_MAX},
        {INT64_MIN, INT64_MAX, -1}, {0, 0, 0}
    };
    for (unsigned i = 0; i < sizeof(cases) / sizeof(cases[0]); ++i)
        if (calculate(cases[i][0], cases[i][1]) != cases[i][2]) return i + 1;
    return 0;
}
"#,
    )
    .unwrap();
    let executable = directory.join("check");
    let linked = Command::new("clang")
        .arg(&driver)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let status = Command::new(&executable).status().unwrap();
    let _ = std::fs::remove_dir_all(directory);
    assert!(status.success(), "failed arithmetic case: {status}");
}

#[test]
fn arm_instruction_contract_rejects_foreign_registers_and_partial_words() {
    let original = source(true);
    for (before, after, code) in [
        ("'x0'", "'rax'", "type-encoding-register"),
        ("'x9'", "'x09'", "type-encoding-register"),
        ("size: 4", "size: 3", "type-encoding-size"),
    ] {
        assert!(original.contains(before));
        let parsed = TokenCursor::parse_source(&original.replace(before, after));
        let mut ctx = TransformCtx::new();
        ctx.compile_target =
            CompileTarget::Concrete(TargetTriple::parse("aarch64-unknown-linux-gnu").unwrap());
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == code),
            "{:?}",
            typed.all_flaws()
        );
    }
}
