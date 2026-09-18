use ast::prelude::Intern;
use ast::{BindValue, Expr};
use flask::{CompileTarget, TargetTriple};
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, EffectTarget, FileId, GroupId};

#[test]
fn declaration_assembly_is_rejected_in_favor_of_expression_form() {
    let source = "protocol := 'gin-asm 1\\ntarget \\\"x86_64-unknown-linux-gnu\\\"\\ndialect intel\\nresult fixed \\\"rax\\\"\\ntemplate \\\"nop\\\"'\n#asm(protocol)\nwrite() Int\n";
    let file = TokenCursor::parse_source(source);
    assert!(
        file.parse_warnings
            .iter()
            .any(|flaw| flaw.code.slug() == "parse-asm-declaration-migration")
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").expect("target"));
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed
            .defs
            .get(&DefId(Intern::from_ref("write")))
            .unwrap()
            .unassigned_decl
    );
}

fn assembly_flaws(source: &str, target: &str) -> Vec<String> {
    let file = TokenCursor::parse_source(source);
    let mut context = TransformCtx::new();
    context.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).expect("target"));
    let typed = transform(&file, FileId(0), &context);
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.message.clone())
        .collect()
}

#[test]
fn raw_address_memory_access_remains_conservative() {
    let file = TokenCursor::parse_source(
        "Len is in 0 to 18446744073709551615\nraw(pointer @Int, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory write pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"', pointer: pointer, length: length)\n",
    );
    assert!(
        file.parse_warnings.is_empty(),
        "parse warnings: {:?}",
        file.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed.defs[&DefId(Intern::from_ref("raw"))]
            .effects
            .unknown_memory
    );
}

#[test]
fn assembly_memory_effects_follow_safe_reference_identity() {
    let file = TokenCursor::parse_source(
        "Len is in 0 to 18446744073709551615\nread_write(pointer ref Int, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory read pointer=\"pointer\" length=\"length\"\\nmemory write pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"', pointer: pointer, length: length)\n",
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let effects = &typed.defs[&DefId(Intern::from_ref("read_write"))].effects;
    let pointee = EffectTarget::Group(GroupId(0));
    assert!(effects.reads.contains(&pointee), "{effects:?}");
    assert_eq!(effects.writes, [pointee.clone()].into_iter().collect());
    assert_eq!(effects.invalidates, [pointee].into_iter().collect());
    assert!(effects.unknown_memory, "{effects:?}");
}

#[test]
fn assembly_memory_effects_substitute_wrapper_parameters() {
    let file = TokenCursor::parse_source(
        "Len is in 0 to 18446744073709551615\nread_write(pointer ref Int, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory write pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"', pointer: pointer, length: length)\nwrapper(amount Len, other ref Int): read_write(ref other, amount)\n",
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let effects = &typed.defs[&DefId(Intern::from_ref("wrapper"))].effects;
    let pointee = EffectTarget::Group(GroupId(0));
    assert_eq!(effects.writes, [pointee.clone()].into_iter().collect());
    assert_eq!(effects.invalidates, [pointee].into_iter().collect());
}

#[test]
fn assembly_read_only_effect_is_only_safe_reference_pointee_read() {
    let file = TokenCursor::parse_source(
        "Len is in 0 to 18446744073709551615\nread(pointer ref Int, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory read pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"', pointer: pointer, length: length)\n",
    );
    assert!(
        file.parse_warnings.is_empty(),
        "parse warnings: {:?}",
        file.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed.all_flaws().is_empty(),
        "unexpected flaws: {:?}",
        typed.all_flaws()
    );
    let effects = &typed.defs[&DefId(Intern::from_ref("read"))].effects;
    let pointee = EffectTarget::Group(GroupId(0));
    assert_eq!(effects.reads, [pointee].into_iter().collect());
    assert!(effects.writes.is_empty(), "{effects:?}");
    assert!(effects.invalidates.is_empty(), "{effects:?}");
}

#[test]
fn assembly_write_only_effect_does_not_imply_read() {
    let file = TokenCursor::parse_source(
        "Len is in 0 to 18446744073709551615\nwrite(pointer ref Int, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory write pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"', pointer: pointer, length: length)\n",
    );
    assert!(
        file.parse_warnings.is_empty(),
        "parse warnings: {:?}",
        file.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed.all_flaws().is_empty(),
        "unexpected flaws: {:?}",
        typed.all_flaws()
    );
    let effects = &typed.defs[&DefId(Intern::from_ref("write"))].effects;
    let pointee = EffectTarget::Group(GroupId(0));
    assert_eq!(
        effects.reads,
        [EffectTarget::Group(GroupId(0))].into_iter().collect()
    );
    assert_eq!(effects.writes, [pointee.clone()].into_iter().collect());
    assert_eq!(effects.invalidates, [pointee].into_iter().collect());
}

#[test]
fn assembly_pointee_write_allows_distinct_mutable_references() {
    let flaws = assembly_flaws(
        "Word is in 0 to 18446744073709551615\nLen is in 0 to 18446744073709551615\nwrite(value ref Word, length Len): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"value\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory write pointer=\"value\" length=\"length\"\\ntemplate \"nop\"', value: value, length: length)\nmain(left mut Word, right mut Word, length Len): write(ref left, length)\n",
        "x86_64-unknown-linux-gnu",
    );
    assert!(flaws.is_empty(), "unexpected aliasing flaw: {flaws:?}");
}

#[test]
fn register_only_ties_do_not_imply_memory_or_ownership_effects() {
    let file = TokenCursor::parse_source(
        "Word is in 0 to 18446744073709551615\ninc(value Word) Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"value\" tied \"rax\"\\nresult fixed \"rax\"\\ntemplate \"inc rax\"', value: value)\n",
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed.all_flaws().is_empty(),
        "unexpected flaws: {:?}",
        typed.all_flaws()
    );
    let effects = &typed.defs[&DefId(Intern::from_ref("inc"))].effects;
    assert!(effects.writes.is_empty(), "{effects:?}");
    assert!(effects.invalidates.is_empty(), "{effects:?}");
    assert!(effects.consumes.is_empty(), "{effects:?}");
}

#[test]
fn safe_reference_register_only_tie_does_not_imply_memory_or_ownership_effects() {
    let source = "Word is in 0 to 18446744073709551615\nidentity(value ref Word) Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"value\" tied \"rax\"\\nresult fixed \"rax\"\\ntemplate \"mov rax, rax\"', value: value)\n";
    let file = TokenCursor::parse_source(source);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed.all_flaws().is_empty(),
        "unexpected flaws: {:?}",
        typed.all_flaws()
    );
    assert!(
        typed.exprs.kind.iter().any(|kind| matches!(
            kind,
            typecheck::TypedExprKind::Asm {
                state: typecheck::assembly::AssemblyState::Checked(_),
                ..
            }
        )),
        "assembly was not checked"
    );
    let effects = &typed.defs[&DefId(Intern::from_ref("identity"))].effects;
    assert!(effects.writes.is_empty(), "{effects:?}");
    assert!(effects.invalidates.is_empty(), "{effects:?}");
    assert!(effects.consumes.is_empty(), "{effects:?}");
}

#[test]
fn checked_assembly_reports_target_and_operand_conflicts() {
    let source = "write(value Int): #asm('gin-asm 1\\ntarget \"aarch64-unknown-linux-gnu\"\\ndialect att\\ninput \"missing\" fixed \"x0\"\\ninput \"value\" fixed \"x0\"\\nresult fixed \"x0\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("does not match active target"))
    );
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("missing assembly operand"))
    );
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("duplicate assembly register"))
    );
}

#[test]
fn checked_assembly_reports_result_and_tie_conflicts() {
    let source = "write(value Int) Int: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" tied \"rcx\"\\nresult fixed \"rax\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("must use result register"))
    );
}

#[test]
fn checked_assembly_rejects_tied_input_without_result() {
    let source = "write(value Int): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" tied \"rax\"\\ntemplate \"inc rax\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("requires a result"))
    );
}

#[test]
fn checked_assembly_rejects_missing_memory_bindings() {
    let source = "write(value Int): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\nmemory read pointer=\"missing_buf\" length=\"missing_len\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("unknown memory pointer"))
    );
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("unknown memory length"))
    );
}

#[test]
fn checked_assembly_rejects_invalid_memory_representations() {
    let source = "write(value Int): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\nmemory read pointer=\"value\" length=\"value\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("memory pointer"))
    );
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("memory length"))
    );
}

#[test]
fn checked_assembly_rejects_unknown_registers() {
    let source = "write(value Int): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" fixed \"not_a_register\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("register") && message.contains("unsupported"))
    );
}

#[test]
fn checked_assembly_rejects_incompatible_register_width() {
    let source = "Word is in 0 to 18446744073709551615\nwrite(value Word): #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" fixed \"eax\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("incompatible with register"))
    );
}

#[test]
fn checked_assembly_rejects_unsupported_target_dialect() {
    let source = "write(): #asm('gin-asm 1\\ntarget \"wasm32-unknown-unknown\"\\ndialect att\\ntemplate \"nop\"')\n";
    let flaws = assembly_flaws(source, "wasm32-unknown-unknown");
    assert!(flaws.iter().any(|message| message.contains("unsupported")));
}

#[test]
fn checked_assembly_reports_missing_target_implementation() {
    let source = "write(): #asm('')\n";
    let flaws = assembly_flaws(source, "wasm32-unknown-unknown");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("no checked assembly implementation"))
    );
}

#[test]
fn expression_assembly_is_checked_and_keeps_named_runtime_operand() {
    let source = "write(value Int) Int: #asm('gin-asm 1\\ntarget \\\"x86_64-unknown-linux-gnu\\\"\\ndialect intel\\ninput \\\"value\\\" fixed \\\"rdi\\\"\\nresult fixed \\\"rax\\\"\\ntemplate \\\"nop\\\"', value: value)";
    let file = TokenCursor::parse_source(source);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    let write = typed.defs.get(&DefId(Intern::from_ref("write"))).unwrap();
    let BindBody::Expr(expr) = write.body else {
        panic!("expected expression body");
    };
    assert!(matches!(
        typed.exprs.kind_of(expr),
        Some(typecheck::TypedExprKind::Asm { operands, .. }) if operands.len() == 1
    ));
}

#[test]
fn checked_assembly_accepts_named_multiple_outputs() {
    let source = "Word is in 0 to 18446744073709551615\nwrite(value Word) sum Word, overflow Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" fixed \"rdi\"\\noutput \"sum\" fixed \"rax\"\\noutput \"overflow\" fixed \"rdx\"\\ntemplate \"add\"', value: value)";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn checked_assembly_accepts_one_named_scalar_output() {
    let source = "Word is in 0 to 18446744073709551615\nread() value Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\noutput \"value\" fixed \"rax\"\\ntemplate \"nop\"')";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn checked_assembly_accepts_special_clobbers_on_supported_targets() {
    for (target, dialect, physical) in [
        ("x86_64-unknown-linux-gnu", "intel", "rax"),
        ("aarch64-apple-darwin", "att", "x16"),
    ] {
        let source = format!(
            "read(): #asm('gin-asm 1\\ntarget \"{target}\"\\ndialect {dialect}\\nclobber \"{physical}\"\\nclobber \"memory\"\\nclobber \"cc\"\\ntemplate \"nop\"')"
        );
        assert!(TokenCursor::parse_source(&source).parse_warnings.is_empty());
        let flaws = assembly_flaws(&source, target);
        assert!(flaws.is_empty(), "{target}: {flaws:?}");
    }
}

#[test]
fn checked_assembly_rejects_special_clobbers_in_operand_roles() {
    for (target, dialect) in [
        ("x86_64-unknown-linux-gnu", "intel"),
        ("aarch64-apple-darwin", "att"),
    ] {
        for register in ["memory", "cc"] {
            for (signature, operand, arguments) in [
                ("read(value Word)", "input \"value\"", ", value: value"),
                ("read() Word", "result", ""),
                ("read() value Word", "output \"value\"", ""),
            ] {
                let source = format!(
                    "Word is in 0 to 18446744073709551615\n{signature}: #asm('gin-asm 1\\ntarget \"{target}\"\\ndialect {dialect}\\n{operand} fixed \"{register}\"\\ntemplate \"nop\"'{arguments})"
                );
                assert!(TokenCursor::parse_source(&source).parse_warnings.is_empty());
                let flaws = assembly_flaws(&source, target);
                assert_eq!(
                    flaws,
                    vec![format!(
                        "assembly register `{register}` is unsupported for target `{target}`"
                    )],
                    "{signature}: {flaws:?}"
                );
            }
        }
    }
}

#[test]
fn checked_assembly_rejects_missing_duplicate_and_unknown_outputs() {
    let source = "Word is in 0 to 18446744073709551615\nwrite() sum Word, carry Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\noutput \"sum\" fixed \"rax\"\\noutput \"sum\" fixed \"rdx\"\\noutput \"other\" fixed \"rcx\"\\ntemplate \"nop\"')\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|m| m.contains("duplicate assembly output `sum`"))
    );
    assert!(
        flaws
            .iter()
            .any(|m| m.contains("unknown assembly output `other`"))
    );
    assert!(
        flaws
            .iter()
            .any(|m| m.contains("missing assembly output `carry`"))
    );
}

#[test]
fn checked_assembly_rejects_named_output_width_and_invalid_tie() {
    let source = "Word is in 0 to 18446744073709551615\nwrite(value Word) sum Word, carry Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"value\" tied \"rcx\"\\noutput \"sum\" fixed \"eax\"\\noutput \"carry\" fixed \"rdx\"\\ntemplate \"nop\"', value: value)\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|m| m.contains("output `sum`") && m.contains("incompatible with register"))
    );
    assert!(
        flaws
            .iter()
            .any(|m| m.contains("tied input `value`") && m.contains("declared output register"))
    );
}

#[test]
fn checked_assembly_rejects_multiple_inputs_tied_to_one_output() {
    let source = "Word is in 0 to 18446744073709551615\\nwrite(lhs Word, rhs Word) sum Word, carry Word: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\ninput \"lhs\" tied \"rax\"\\ninput \"rhs\" tied \"rax\"\\noutput \"sum\" fixed \"rax\"\\noutput \"carry\" fixed \"rdx\"\\ntemplate \"add\"', lhs: lhs, rhs: rhs)\\n";
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(
        flaws
            .iter()
            .any(|message| message.contains("multiple tied inputs target assembly output 0")),
        "{flaws:?}"
    );
}

#[test]
fn target_expression_uses_the_selected_concrete_target() {
    let file = TokenCursor::parse_source("arch := #target.arch\n");
    let bind = file.defs.get(&Intern::from_ref("arch")).unwrap();
    let BindValue::Expr(expr) = &bind.value else {
        panic!("expected constant expression");
    };
    let target = CompileTarget::Concrete(TargetTriple::parse("aarch64-unknown-linux-gnu").unwrap());
    let value = typecheck::analysis::CompTimeEvaluator::for_target(
        &typecheck::analysis::ConstEnv::default(),
        &file,
        &target,
    )
    .eval(&expr.value);
    assert_eq!(value, Some(ast::ConstValue::Text("arm64".into())));
    assert!(matches!(expr.value, Expr::RecordGet { .. }));
}

#[test]
fn user_defined_builder_function_produces_checked_assembly_specification() {
    let source = "#inline\nbuilder() String := when #target.arch is 'x86_64' then 'gin-asm 1\\ntarget \\\"x86_64-unknown-linux-gnu\\\"\\ndialect intel\\ntemplate \\\"nop\\\"' else 'gin-asm 1\\ntarget \\\"aarch64-unknown-linux-gnu\\\"\\ndialect att\\ntemplate \\\"nop\\\"'\nwrite(): #asm(builder())\n";
    let file = TokenCursor::parse_source(source);
    let write = file.defs.get(&Intern::from_ref("write")).unwrap();
    let BindValue::Expr(body) = &write.value else {
        panic!("expected body");
    };
    let Expr::Asm { specification, .. } = &body.value else {
        panic!("expected asm");
    };
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let value = typecheck::analysis::CompTimeEvaluator::for_target(
        &typecheck::analysis::ConstEnv::default(),
        &file,
        &target,
    )
    .eval(&specification.value);
    assert!(
        matches!(value, Some(ast::ConstValue::Text(spec)) if spec.contains("x86_64-unknown-linux-gnu"))
    );
}

#[test]
fn record_builder_is_checked_after_ctfe() {
    let source = r#"Word is in 0 to 18446744073709551615
String has value Int
#infix('+')
concat(lhs String, rhs String) String: lhs
Assembly has protocol String
#inline
new_assembly(value String) Assembly: return Assembly(protocol: value)
#inline
finish(assembly own Assembly) String: return assembly.protocol
#inline
builder() String := finish(new_assembly('gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"value\" fixed \"rdi\"\nresult fixed \"rax\"\ntemplate \"nop\"'))
write(value Word) Word: #asm(builder(), value: value)
"#;
    let file = TokenCursor::parse_source(source);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    let write = typed.defs.get(&DefId(Intern::from_ref("write"))).unwrap();
    let BindBody::Expr(expr) = write.body else {
        panic!("expected expression")
    };
    assert!(
        matches!(
            typed.exprs.kind_of(expr),
            Some(typecheck::TypedExprKind::Asm {
                state: typecheck::assembly::AssemblyState::Checked(_),
                ..
            })
        ),
        "{:#?}",
        typed.all_flaws()
    );
}

#[test]
fn library_target_defers_assembly_realization() {
    let file = TokenCursor::parse_source(
        "write() Int: #asm('gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect intel\\nresult fixed \"rax\"\\ntemplate \"nop\"')",
    );
    let context = TransformCtx::new();
    let typed = transform(&file, FileId(0), &context);
    let write = typed
        .defs
        .get(&DefId(Intern::from_ref("write")))
        .unwrap_or_else(|| {
            panic!(
                "write missing: parse={:#?}, typed={:#?}",
                file.parse_warnings,
                typed.all_flaws()
            )
        });
    let BindBody::Expr(expr) = write.body else {
        panic!("expected expression body")
    };
    assert!(matches!(
        typed.exprs.kind_of(expr),
        Some(typecheck::TypedExprKind::Asm {
            state: typecheck::assembly::AssemblyState::Pending,
            ..
        })
    ));
}

#[test]
fn runtime_assembly_specification_reports_stage_boundary() {
    let file = TokenCursor::parse_source(
        "spec(value String) String: return value\nwrite(value String): #asm(spec(value))",
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-cannot-call-comptime-with-runtime-args" })
    );
}

#[test]
fn compile_time_text_assembly_is_checked() {
    let source = r#"protocol := 'gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\nresult fixed "rax"\ntemplate "nop"'
write() Int: #asm(protocol)
"#;
    let file = TokenCursor::parse_source(source);
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let typed = transform(&file, FileId(0), &context);
    let write = typed.defs.get(&DefId(Intern::from_ref("write"))).unwrap();
    let BindBody::Expr(expr) = write.body else {
        panic!("expected expression body")
    };
    assert!(
        matches!(
            typed.exprs.kind_of(expr),
            Some(typecheck::TypedExprKind::Asm {
                state: typecheck::assembly::AssemblyState::Checked(_),
                ..
            })
        ),
        "{:#?}",
        typed.all_flaws()
    );
}

#[test]
fn runtime_text_assembly_is_rejected_at_asm_boundary() {
    let source = r#"Byte is in 0 to 255
write(spec [Byte; 8]): #asm(spec)
"#;
    let flaws = assembly_flaws(source, "x86_64-unknown-linux-gnu");
    assert!(flaws.iter().any(|message| message.contains("compile-time")));
}
