use flask::{CompileTarget, TargetTriple};
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let file = TokenCursor::parse_source(source);
    assert!(
        file.parse_warnings.is_empty(),
        "parse warnings: {:?}",
        file.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").expect("target"));
    let typed = transform(&file, FileId(0), &context);
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
    typed
}

const ASM: &str = "'gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory write pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"'";
const READ_ASM: &str = "'gin-asm 1\\ntarget \"x86_64-unknown-linux-gnu\"\\ndialect att\\ninput \"pointer\" fixed \"rdi\"\\ninput \"length\" fixed \"rsi\"\\nmemory read pointer=\"pointer\" length=\"length\"\\ntemplate \"nop\"'";

#[test]
fn assembly_reader_allows_shared_aliases() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nreader(length Len, pointer ref{{shared}} Word): #asm({READ_ASM}, pointer: pointer, length: length)\nwrapper(length Len, left ref{{shared}} Word, right ref{{shared}} Word): reader(length, ref left)\nmain(value mut Word, length Len): wrapper(length, ref value, ref value)\n"
    );
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn assembly_wrapper_rejects_shared_mut_aliases() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nwrite(length Len, pointer ref{{shared}} Word): #asm({ASM}, pointer: pointer, length: length)\nwrapper(length Len, left ref{{shared}} Word, right mut{{shared}} Word): write(length, ref left)\nmain(value mut Word, length Len): wrapper(length, ref value, mut value)\n"
    );
    let typed = check(&source);
    let flaws = typed.all_flaws();
    assert!(!flaws.is_empty());
    assert!(
        flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() == "type-overlapping-target-group-arguments")
    );
}

#[test]
fn assembly_write_preserves_unrelated_reference_on_reuse() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nwrite(pointer ref Word, length Len): #asm({ASM}, pointer: pointer, length: length)\nmain(left mut Word, right mut Word, length Len) Word:\n    ref saved Word: right\n    write(ref left, length)\n    result: saved\n    return 0\n"
    );
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn assembly_write_invalidates_affected_reference_on_reuse() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nwrite(pointer ref Word, length Len): #asm({ASM}, pointer: pointer, length: length)\nmain(length Len) Word:\n    items: [0; 2]\n    ref saved Word: items.(0)\n    write(ref items.(0), length)\n    result: saved\n    return 0\n"
    );
    let typed = check(&source);
    let flaws = typed.all_flaws();
    assert!(
        !flaws.is_empty()
            && flaws
                .iter()
                .all(|(_, flaw)| flaw.code.slug() == "type-use-of-invalidated-reference")
    );
}

#[test]
fn direct_assembly_write_invalidates_affected_reference_on_reuse() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nmain(length Len) Word:\n    items: [0; 2]\n    ref saved Word: items.(0)\n    written: #asm({ASM}, pointer: ref items.(0), length: length)\n    result: saved\n    return 0\n"
    );
    let typed = check(&source);
    let flaws = typed.all_flaws();
    assert!(!flaws.is_empty());
    assert!(
        flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() == "type-use-of-invalidated-reference")
    );
}

#[test]
fn direct_assembly_write_preserves_unrelated_reference_on_reuse() {
    let source = format!(
        "Len is in 0 to 18446744073709551615\nWord is in 0 to 255\nmain(length Len) Word:\n    items: [0; 2]\n    ref saved Word: items.(1)\n    written: #asm({ASM}, pointer: ref items.(0), length: length)\n    result: saved\n    return 0\n"
    );
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
