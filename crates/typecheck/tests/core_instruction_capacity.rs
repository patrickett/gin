mod support;

use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn check(source: &str) -> typecheck::TypedFileAst {
    let range = include_str!("../../../modules/core/range/range.gin");
    let instructions = include_str!("../../../modules/core/target/emission/emission.gin")
        .replace("use core.range Range\n", "");
    let parsed = format!("{range}\n{instructions}\n{source}").parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let context = TransformCtx::new();
    let mut files = [parsed.ast];
    let diagnostics = typecheck::prepare_package_asts(&mut files, &context.compile_target);
    assert!(diagnostics.iter().all(Vec::is_empty), "{diagnostics:?}");
    transform(&files[0], FileId(0), &context)
}

#[test]
fn instruction_writes_prove_remaining_capacity_and_publish_the_new_length() {
    for (operation, start, result) in [("byte", 1023, 1024), ("word", 1020, 1024)] {
        let source = format!(
            "append(buffer mut Instructions and buffer.length = {start}):
    Instructions.{operation}(buffer, 0)
    return thus buffer.length = {result}
"
        );
        let typed = check(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}\n{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn instruction_writes_reject_full_and_partially_full_buffers() {
    for (operation, start) in [("byte", 1024), ("word", 1021)] {
        let source = format!(
            "append(buffer mut Instructions and buffer.length = {start}):
    Instructions.{operation}(buffer, 0)
    return
"
        );
        let typed = check(&source);
        let codes = support::flaw_codes(&typed);
        assert!(
            !codes.is_empty()
                && codes
                    .iter()
                    .all(|code| code.starts_with("type-parameter-refinement-")),
            "{source}\n{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn instruction_patch_requires_a_written_position() {
    for (length, position, accepted) in [
        (0, 0, false),
        (4, 0, true),
        (1024, 1023, true),
        (1024, 1024, false),
    ] {
        let source = format!(
            "patch(buffer mut Instructions and buffer.length = {length}):
    Instructions.patch(buffer, {position}, 0)
    return
"
        );
        let typed = check(&source);
        assert_eq!(
            typed.all_flaws().is_empty(),
            accepted,
            "{source}\n{:?}",
            typed.all_flaws()
        );
        if !accepted {
            assert!(
                support::flaw_codes(&typed)
                    .iter()
                    .all(|code| code.starts_with("type-parameter-refinement-")),
                "{:?}",
                typed.all_flaws()
            );
        }
    }
}
