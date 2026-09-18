use flask::{CompileTarget, TargetTriple};
use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

#[test]
fn target_selection_permits_alternatives_for_other_builds() {
    let source = "Byte is in 0 to 255\nchoose() Byte := when #target.arch is\n    'x86_64' then (1 as Byte)\n    else (2 as Byte)\n";
    for target in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
        let mut ctx = TransformCtx::new();
        ctx.compile_target = CompileTarget::Concrete(TargetTriple::parse(target).unwrap());
        let parsed = TokenCursor::parse_source(source);
        assert!(
            parsed.parse_warnings.is_empty(),
            "{:?}",
            parsed.parse_warnings
        );
        let typed = transform(&parsed, FileId(0), &ctx);
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let ordinary = TokenCursor::parse_source(
            "Byte is in 0 to 255\nchoose() Byte := when 1 is\n    0 then (1 as Byte)\n    else (2 as Byte)\n",
        );
        let typed = transform(&ordinary, FileId(0), &ctx);
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug() == "type-unreachable-when-arm")
        );
    }
}
