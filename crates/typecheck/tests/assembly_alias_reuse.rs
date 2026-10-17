use flask::{CompileTarget, TargetTriple};
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let metadata = r#"
#phantom(input, output)
MemorySignature(input, output) has
    version Len
    finish(self):
        return
memory_signature MemorySignature([pointer ref Word, length Len], ()): MemorySignature(version: 1)
write_code := [signature: memory_signature, target: #target.triple, bytes: [144], inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], length: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [], clobbers: [], reads: [], writes: [[pointer: 0, length: 1]]]
read_code := [signature: memory_signature, target: #target.triple, bytes: [144], inputs: [pointer: [architecture: "x86_64", bank: "general", index: 7, width: 64], length: [architecture: "x86_64", bank: "general", index: 6, width: 64]], outputs: [], clobbers: [], reads: [[pointer: 0, length: 1]], writes: []]
"#;
    let file = TokenCursor::parse_source(&format!("{source}\n{metadata}"));
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
            typecheck::TypedExprKind::Emit {
                state: typecheck::assembly::AssemblyState::Checked(_),
                ..
            }
        )),
        "emission was not checked: {:?}",
        typed.all_flaws()
    );
    typed
}

#[test]
fn assembly_reader_allows_shared_aliases() {
    let source = String::from(format!(
        "Len is in 0 to 18446744073709551615\nLen has\n    finish(self):\n        return\n{}reader(pointer ref{{shared}} Word, length Len): return #emit(read_code, [pointer: pointer, length: length])\nwrapper(length Len, left ref{{shared}} Word, right ref{{shared}} Word): return reader(left, length)\nmain(value mut Word, length Len): return wrapper(length, value, value)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn assembly_wrapper_rejects_shared_mut_aliases() {
    let source = String::from(format!(
        "Len is in 0 to 18446744073709551615\nLen has\n    finish(self):\n        return\n{}write(pointer ref{{shared}} Word, length Len): return #emit(write_code, [pointer: pointer, length: length])\nwrapper(length Len, left ref{{shared}} Word, right mut{{shared}} Word): return write(left, length)\nmain(value mut Word, length Len): return wrapper(length, value, value)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
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
    let source = String::from(format!(
        "Len is in 0 to 18446744073709551615\nLen has\n    finish(self):\n        return\n{}write(pointer ref Word, length Len): return #emit(write_code, [pointer: pointer, length: length])\nmain(left mut Word, right mut Word, length Len) Word:\n    ref saved Word: right\n    write(left, length)\n    result: saved\n    return 0\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let typed = check(&source);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn assembly_write_invalidates_affected_reference_on_reuse() {
    let source = String::from(format!(
        "Len is in 0 to 18446744073709551615\nLen has\n    finish(self):\n        return\n{}write(pointer ref Word, length Len): return #emit(write_code, [pointer: pointer, length: length])\nmain(length Len) Word:\n    items: [0; 2]\n    ref saved Word: items.[0]\n    write(items.[0], length)\n    result: saved\n    return 0\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    let typed = check(&source);
    let flaws = typed.all_flaws();
    assert!(
        !flaws.is_empty()
            && flaws
                .iter()
                .all(|(_, flaw)| flaw.code.slug() == "type-use-of-invalidated-reference")
    );
}
