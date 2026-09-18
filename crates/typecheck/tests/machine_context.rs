use flask::{CompileTarget, TargetTriple};
use parser::parse_from_str;
use typecheck::assembly::AssemblyState;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedExprKind, TypedFileAst};

const SOURCE: &str = r#"Word is in -9223372036854775808 to 9223372036854775807

#machine(architecture)
mov(value Word) Word: value

#machine(architecture)
add(left Word, right Word) Word: left + right

#machine(architecture)
add_words(a Word, b Word) Word:
    left: rax
    right: rcx
    left:: mov(a)
    right:: mov(b)
    left:: add(left, right)
return left

calculate(a Word, b Word) Word: add_words(a, b)
"#;

fn check(source: &str, target: &str) -> TypedFileAst {
    let backend = test_fixtures::machine_backend_source(&target);
    check_backend(source, backend, target)
}

fn check_backend(source: &str, backend: &str, target: &str) -> TypedFileAst {
    let source = format!("{source}\n{backend}");
    let file = parse_from_str(&source);
    assert!(file.parse_warnings.is_empty(), "{:?}", file.parse_warnings);
    let mut ctx = TransformCtx::new();
    ctx.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
    transform(&file, FileId(0), &ctx)
}

#[test]
fn register_helpers_expand_into_one_checked_region() {
    let typed = check(SOURCE, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let assembly = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Asm {
                state: AssemblyState::Checked(assembly),
                ..
            } if assembly.template.contains("mov rcx") => Some(assembly),
            _ => None,
        })
        .expect("expanded region");
    assert_eq!(
        assembly.template,
        "mov rax, rdi\nmov rcx, rsi\nadd rax, rcx"
    );
    assert_eq!(assembly.result.as_deref(), Some("rax"));
    assert!(assembly.clobbers.iter().any(|register| register == "rcx"));
    assert!(assembly.clobbers.iter().any(|register| register == "cc"));
}

#[test]
fn arm_register_helpers_use_three_operand_addition() {
    let source = SOURCE.replace("rax", "x9").replace("rcx", "x10");
    let typed = check(&source, "aarch64-apple-darwin");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::Asm { state: AssemblyState::Checked(assembly), .. }
            if assembly.template == "mov x9, x0\nmov x10, x1\nadd x9, x9, x10"
    )));
}

#[test]
fn rejects_invalid_machine_programs() {
    for (source, message) in [
        (
            SOURCE.replace("#machine(architecture)\nadd(", "add("),
            "callee must also have #machine",
        ),
        (
            SOURCE.replace("left:: mov(a)", ""),
            "read before initialization",
        ),
        (SOURCE.replace("right: rcx", "right: rax"), "overlaps"),
        (
            SOURCE.replace("left:: add(left, right)", "left:: add(right, left)"),
            "destination to be the left operand",
        ),
        (
            SOURCE.replace("left + right", "add(left, right)"),
            "recursive",
        ),
        (
            SOURCE.replace("left + right", "left * right"),
            "copies and addition",
        ),
        (
            SOURCE.replace("left:: add(left, right)", "left:: add(left)"),
            "expects 2 arguments",
        ),
        (
            SOURCE.replace("right: rcx", "right: rsp"),
            "not an available scratch register",
        ),
        (
            SOURCE.replace("left: rax", "left Word: rax"),
            "unannotated register bindings",
        ),
    ] {
        let typed = check(&source, "x86_64-unknown-linux-gnu");
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains(message)),
            "expected {message}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn rejects_types_that_do_not_cover_a_full_machine_word() {
    for range in ["0 to 255", "0 to 9223372036854775807"] {
        let source = SOURCE.replace("-9223372036854775808 to 9223372036854775807", range);
        let typed = check(&source, "x86_64-unknown-linux-gnu");
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains("full signed or unsigned 64-bit")),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn machine_helpers_expand_across_package_files() {
    use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

    let (helpers, caller) = SOURCE
        .split_once("#machine(architecture)\nadd_words")
        .unwrap();
    let caller = format!("#machine(architecture)\nadd_words{caller}");
    let artifacts = transform_package_with_shared_context(
        vec![
            parse_from_str(&format!(
                "{helpers}\n{}",
                test_fixtures::machine_backend_source("x86_64")
            )),
            parse_from_str(&caller),
        ],
        PackageTransformOptions::default().with_compile_target(CompileTarget::Concrete(
            TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
        )),
    );
    for typed in &artifacts.typed_asts {
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

const MUTABLE_SOURCE: &str = r#"Word is in -9223372036854775808 to 9223372036854775807

#machine(architecture)
mov(destination mut Word, source Word):
    destination:: source
return

#machine(architecture)
add(destination mut Word, source Word):
    destination:: destination + source
return

#machine(architecture)
add_words(a Word, b Word) Word:
    left: rax
    right: rcx
    input := b
    mov(left, a)
    mov(right, input)
    add(left, right)
return left

calculate(a Word, b Word) Word: add_words(a, b)
"#;

#[test]
fn mutable_helpers_infer_argument_access_and_expand_without_calls() {
    for (target, source, expected) in [
        (
            "x86_64-unknown-linux-gnu",
            MUTABLE_SOURCE.to_string(),
            "mov rax, rdi\nmov rcx, rsi\nadd rax, rcx",
        ),
        (
            "aarch64-apple-darwin",
            MUTABLE_SOURCE.replace("rax", "x9").replace("rcx", "x10"),
            "mov x9, x0\nmov x10, x1\nadd x9, x9, x10",
        ),
    ] {
        let typed = check(&source, target);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
            TypedExprKind::Asm { state: AssemblyState::Checked(assembly), .. }
                if assembly.template == expected
        )));
    }
}

#[test]
fn rejects_mutation_through_immutable_bindings_and_overlapping_arguments() {
    for (source, message) in [
        (
            MUTABLE_SOURCE.replace("add(left, right)", "add(input, right)"),
            "immutable register",
        ),
        (
            MUTABLE_SOURCE.replace("add(left, right)", "input:: right"),
            "immutable register",
        ),
        (
            MUTABLE_SOURCE.replace("add(left, right)", "saved := left\n    add(left, right)"),
            "immutable register",
        ),
        (
            MUTABLE_SOURCE.replace("add(left, right)", "add(left, left)"),
            "overlapping machine arguments",
        ),
        (
            MUTABLE_SOURCE.replace("mov(left, a)", ""),
            "read before initialization",
        ),
        (
            MUTABLE_SOURCE.replace("input := b", "input := left"),
            "read before initialization",
        ),
        (
            MUTABLE_SOURCE.replace("input := b", "input := rax"),
            "initialized register",
        ),
        (
            MUTABLE_SOURCE.replace("add(left, right)", "left:: mov(left, right)"),
            "unit machine helper",
        ),
        (
            MUTABLE_SOURCE.replace("mov(left, a)", "mov(mut left, a)"),
            "mutable register binding",
        ),
    ] {
        let typed = check(&source, "x86_64-unknown-linux-gnu");
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains(message)),
            "expected {message}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn mutable_helper_boundary_declares_memory_reads_and_writes() {
    let typed = check(MUTABLE_SOURCE, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for (name, reads) in [("mov", false), ("add", true)] {
        let bind = &typed.defs[&typecheck::DefId(ast::prelude::Intern::from_ref(name))];
        let typecheck::BindBody::Expr(expr) = bind.body else {
            panic!("assembly body")
        };
        let TypedExprKind::Asm {
            state: AssemblyState::Checked(assembly),
            ..
        } = &typed.exprs.kind[expr.as_usize()]
        else {
            panic!("checked assembly")
        };
        assert_eq!(!assembly.memory.reads.is_empty(), reads);
        assert_eq!(assembly.memory.writes.len(), 1);
        assert_eq!(assembly.memory.writes[0].0, "destination");
        assert!(assembly.result.is_none());
        assert!(assembly.template.ends_with("mov qword ptr [rdi], rax"));
    }
}

#[test]
fn mutable_helpers_forward_registers_and_scope_immutable_aliases() {
    let source = MUTABLE_SOURCE.replace("    add(left, right)", "    twice(left, right)");
    let source = format!(
        "{source}\n#machine(architecture)\ntwice(destination mut Word, source Word):\n    input := source\n    add(destination, input)\n    add(destination, input)\nreturn\n"
    );
    let typed = check(&source, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::Asm { state: AssemblyState::Checked(assembly), .. }
            if assembly.template == "mov rax, rdi\nmov rcx, rsi\nadd rax, rcx\nadd rax, rcx"
    )));
}

#[test]
fn helper_local_immutable_alias_does_not_freeze_callers_register_after_return() {
    let source = MUTABLE_SOURCE.replace(
        "    add(left, right)",
        "    left:: snapshot(left)\n    add(left, right)",
    );
    let source = format!(
        "{source}\n#machine(architecture)\nsnapshot(value Word) Word:\n    saved := value\nreturn saved\n"
    );
    let typed = check(&source, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn backend_values_control_registers_and_instruction_templates() {
    let backend = test_fixtures::machine_backend_source("x86_64")
        .replace(
            r#"(inputs0: "rdi", inputs1: "rsi")"#,
            r#"(inputs0: "r8", inputs1: "r9")"#,
        )
        .replace(
            r#""mov {destination}, {left}""#,
            r#""lea {destination}, [{left}]""#,
        );
    let typed = check_backend(SOURCE, &backend, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::Asm { state: AssemblyState::Checked(assembly), .. }
            if assembly.template == "lea rax, [r8]\nlea rcx, [r9]\nadd rax, rcx"
    )));
}

#[test]
fn rejects_invalid_backend_contracts() {
    let backend = test_fixtures::machine_backend_source("x86_64");
    for (backend, expected) in [
        (
            backend.replace(r#"arch: "x86_64""#, r#"arch: "arm64""#),
            "does not match",
        ),
        (
            backend.replace(
                r#"(inputs0: "rdi", inputs1: "rsi")"#,
                r#"(inputs0: "rdi", inputs1: "rdi")"#,
            ),
            "overlap",
        ),
        (
            backend.replace(
                r#"(inputs0: "rdi", inputs1: "rsi")"#,
                r#"(inputs0: "rax", inputs1: "rsi")"#,
            ),
            "overlap",
        ),
        (backend.replace("{destination}", "{unknown}"), "placeholder"),
        (
            backend.replace(
                r#"scratch0: "rax", scratch1: "rcx", scratch2: "rdx""#,
                r#"scratch0: "r10", scratch1: "rcx", scratch2: "rdx""#,
            ),
            "not an available scratch",
        ),
    ] {
        assert_ne!(backend, test_fixtures::machine_backend_source("x86_64"));
        let typed = check_backend(SOURCE, &backend, "x86_64-unknown-linux-gnu");
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains(expected)),
            "expected {expected}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn machine_context_requires_an_explicit_backend() {
    let typed = check(
        &SOURCE.replace("#machine(architecture)", "#machine"),
        "x86_64-unknown-linux-gnu",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("requires one backend value")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn helpers_must_use_the_same_backend_value() {
    let source = SOURCE.replace("#machine(architecture)\nadd(", "#machine(other)\nadd(");
    let typed = check(
        &format!("{source}\nother := architecture\n"),
        "x86_64-unknown-linux-gnu",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());

    let backend = test_fixtures::machine_backend_source("x86_64");
    let (_, definition) = backend.split_once("architecture :=").unwrap();
    let other = format!("other := {definition}").replace(
        r#""mov {destination}, {left}""#,
        r#""lea {destination}, [{left}]""#,
    );
    let typed = check(&format!("{source}\n{other}"), "x86_64-unknown-linux-gnu");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("caller's backend")),
        "{:?}",
        typed.all_flaws()
    );
}

const ASSEMBLY_LEAF: &str = r#"Byte is in 0 to 255
Word is in 0 to 18446744073709551615

#machine(architecture)
transfer(source @Byte, destination @Byte, length Word) @Byte: #asm('gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\ninput "source" fixed "rdi"\ninput "destination" tied "rax"\ninput "length" fixed "rdx"\nresult fixed "rax"\nmemory read pointer="source" length="length"\nmemory write pointer="destination" length="length"\nclobber "rcx"\ntemplate "nop"', source: source, destination: destination, length: length)

copy(source @Byte, destination @Byte, length Word) @Byte: transfer(source, destination, length)
"#;

#[test]
fn assembly_machine_leaves_accept_pointer_results_and_mixed_boundary_parameters() {
    let typed = check(ASSEMBLY_LEAF, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let assembly = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Asm {
                state: AssemblyState::Checked(assembly),
                ..
            } => Some(assembly),
            _ => None,
        })
        .expect("checked assembly leaf");
    assert_eq!(assembly.inputs.len(), 3);
    assert_eq!(assembly.result.as_deref(), Some("rax"));
    assert_eq!(assembly.memory.reads, [("source".into(), "length".into())]);
    assert_eq!(
        assembly.memory.writes,
        [("destination".into(), "length".into())]
    );
    assert!(assembly.clobbers.iter().any(|register| register == "rcx"));
}

#[test]
fn assembly_machine_leaves_accept_mixed_named_results() {
    let source = r#"Word is in 0 to 18446744073709551615
Byte is in 0 to 255
#machine(architecture)
read(value @Byte) low @Byte, wide Word: #asm('gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\ninput "value" fixed "rdi"\noutput "low" fixed "rax"\noutput "wide" fixed "rdx"\ntemplate "nop"', value: value)
"#;
    let typed = check(source, "x86_64-unknown-linux-gnu");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind,
        TypedExprKind::Asm { state: AssemblyState::Checked(assembly), .. }
            if assembly.outputs.len() == 2
    )));
}

#[test]
fn assembly_machine_leaves_keep_backend_and_assembly_validation() {
    for (source, message) in [
        (
            ASSEMBLY_LEAF.replace("#machine(architecture)", "#machine"),
            "requires one backend value",
        ),
        (
            ASSEMBLY_LEAF.replace("length Word", "length Byte"),
            "memory length `length` must have 64-bit",
        ),
        (
            ASSEMBLY_LEAF.replace("fixed \"rdi\"", "fixed \"not_a_register\""),
            "assembly register `not_a_register` is unsupported",
        ),
        (
            ASSEMBLY_LEAF.replace("#machine(architecture)", "#machine(missing)"),
            "missing",
        ),
    ] {
        let typed = check(&source, "x86_64-unknown-linux-gnu");
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.message.contains(message)),
            "expected {message}: {:?}",
            typed.all_flaws()
        );
    }
    let backend = test_fixtures::machine_backend_source("x86_64")
        .replace(r#"arch: "x86_64""#, r#"arch: "arm64""#);
    let typed = check_backend(ASSEMBLY_LEAF, &backend, "x86_64-unknown-linux-gnu");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.message.contains("does not match")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn register_bodies_reject_expanding_assembly_machine_leaves() {
    let source = r#"Word is in -9223372036854775808 to 9223372036854775807
#machine(architecture)
leaf(value Word) Word: #asm('gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\ninput "value" tied "rax"\nresult fixed "rax"\ntemplate "nop"', value: value)
#machine(architecture)
caller(value Word) Word: leaf(value)
"#;
    let typed = check(source, "x86_64-unknown-linux-gnu");
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| flaw
            .message
            .contains("cannot expand assembly machine leaf `leaf`")),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn assembly_leaf_operands_only_call_machine_helpers() {
    let helper = "identity(value Word) Word: value\n";
    let source = ASSEMBLY_LEAF.replace("length: length)", "length: identity(length))");
    let typed = check(&format!("{source}\n{helper}"), "x86_64-unknown-linux-gnu");
    assert!(
        typed.all_flaws().iter().any(|(_, flaw)| flaw
            .message
            .contains("cannot call `identity` in an assembly operand")),
        "{:?}",
        typed.all_flaws()
    );

    let typed = check(
        &format!("{source}\n#machine(architecture)\n{helper}"),
        "x86_64-unknown-linux-gnu",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
