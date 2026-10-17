use flask::{CompileTarget, TargetTriple};
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

#[test]
fn library_defers_target_selected_constants_used_by_runtime_functions() {
    let source = format!(
        "{}selected := when #target.arch is\n    'x86_64' then (1 as Byte)\n    else (2 as Byte)\nread Byte: selected\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    for parsed in [
        TokenCursor::parse_source(&source),
        TokenCursor::parse_source(&source).qualify_module_defs("circuit"),
    ] {
        let typed = transform(&parsed, FileId(0), &TransformCtx::new());
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

#[test]
fn library_defers_layout_query_domain_proofs_until_target_selection() {
    let source = "Word is in -9223372036854775808 to 9223372036854775807\nWord has\n    finish(self):\n        return\nsize:\n    bytes is in Word: #size(Word)\n    return bytes\n";
    let parsed = TokenCursor::parse_source(source);
    for target in [
        CompileTarget::Library,
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap()),
        CompileTarget::Concrete(TargetTriple::parse("aarch64-apple-darwin").unwrap()),
    ] {
        let mut ctx = TransformCtx::new();
        ctx.compile_target = target;
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }

    let parsed = TokenCursor::parse_source(&format!(
        "Tiny is in 0 to 1\n{}",
        source.replace("bytes is in Word", "bytes is in Tiny")
    ));
    let mut ctx = TransformCtx::new();
    ctx.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("aarch64-apple-darwin").unwrap());
    let typed = transform(&parsed, FileId(0), &ctx);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-integer-narrowing-failed"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn target_selection_permits_alternatives_for_other_builds() {
    let source = format!(
        "{}choose() Byte := return when #target.arch is\n    'x86_64' then (1 as Byte)\n    else (2 as Byte)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    for target in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        let mut ctx = TransformCtx::new();
        ctx.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
        let parsed = TokenCursor::parse_source(&source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let ordinary = TokenCursor::parse_source(&format!(
            "{}choose() Byte := return when 1 is\n    0 then (1 as Byte)\n    else (2 as Byte)\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ));
        let typed = transform(&ordinary, FileId(0), &ctx);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-unreachable-when-arm")
        );
    }
}

#[test]
fn preparing_target_dependent_records_preserves_named_constructor_arguments() {
    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let mut files = [TokenCursor::parse_source(&format!(
        "{}Pair has\n    first Byte, second Byte\n    finish(self):\n        return\nselected := when #target.arch is\n    'x86_64' then Pair(first: 1, second: 2)\n    else Pair(first: 3, second: 4)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ))];
    let diagnostics = typecheck::prepare_package_asts(&mut files, &target);
    assert!(diagnostics.iter().all(Vec::is_empty), "{diagnostics:?}");
    let mut context = TransformCtx::new();
    context.compile_target = target;
    let typed = transform(&files[0], FileId(0), &context);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
