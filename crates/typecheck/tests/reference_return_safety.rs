use parser::cursor::TokenCursor;
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

fn check(source: &str, expected: &[&str]) {
    let source = format!("Int is in 0 to 100\n{source}");
    let typed = transform(
        &TokenCursor::parse_source(&source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(
        typed.parse_warnings.is_empty(),
        "{:?}",
        typed.parse_warnings
    );
    let mut codes: Vec<_> = typed
        .all_flaws()
        .iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect();
    codes.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    assert_eq!(codes, expected, "{source}");
}

#[test]
fn local_reference_without_input_group_is_rejected() {
    check(
        "bad() ref Int:\n    value Int: 7\nreturn ref value\n",
        &["type-reference-return-local-target"],
    );
}

#[test]
fn local_reference_on_early_return_is_rejected() {
    check(
        "bad(input ref Int, flag Int) ref Int:\n    value Int: 7\n    if flag is 1\n    return ref value\nreturn input\n",
        &["type-reference-return-local-target"],
    );
}

#[test]
fn wrong_group_on_early_return_is_rejected() {
    check(
        "bad(left ref{a} Int, right ref{b} Int, flag Int) ref{a} Int:\n    if flag is 1\n    return right\nreturn left\n",
        &["type-reference-return-wrong-target-group"],
    );
}

#[test]
fn caller_reference_on_both_paths_is_allowed() {
    check(
        "good(input ref Int, flag Int) ref Int:\n    if flag is 1\n    return input\nreturn input\n",
        &[],
    );
}

#[test]
fn owned_local_return_is_allowed() {
    check("good() Int:\n    value Int: 7\nreturn value\n", &[]);
}

#[test]
fn local_reference_in_named_results_is_rejected() {
    check(
        "bad() value ref Int, number Int:\n    local Int: 7\nreturn ref local, 0\n",
        &["type-reference-return-local-target", "type-return-mismatch"],
    );
}

#[test]
fn caller_reference_through_local_alias_is_allowed() {
    check(
        "good(input ref Int) ref Int:\n    ref saved Int: input\nreturn saved\n",
        &[],
    );
}

#[test]
fn nested_early_local_reference_is_rejected() {
    check(
        "bad(input ref Int, flag Int) ref Int:\n    value Int: 7\n    if flag is 1\n        if flag is 1\n        return ref value\n    return input\nreturn input\n",
        &["type-reference-return-local-target"],
    );
}

#[test]
fn forwarded_named_reference_results_keep_their_groups() {
    check(
        "pair(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return left, right\nforward(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return pair(ref left, ref right)\n",
        &[],
    );
}

#[test]
fn forwarded_named_reference_results_reject_swapped_groups() {
    check(
        "pair(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return left, right\nforward(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return pair(ref right, ref left)\n",
        &["type-reference-return-wrong-target-group"],
    );
}

#[test]
fn reference_to_pointer_projection_is_rejected() {
    check(
        "bad(pointer @Int) ref Int: ref pointer.(0)\n",
        &["type-reference-return-local-target"],
    );
}

#[test]
fn local_reference_in_when_arm_is_rejected() {
    check(
        "bad(input ref Int, flag Int) ref Int:\n    local Int: 7\nreturn when flag is 1 then ref local else input\n",
        &["type-reference-return-local-target"],
    );
}

#[test]
fn caller_reference_in_when_arms_is_allowed() {
    check(
        "good(input ref Int, flag Int) ref Int: when flag is 1 then input else input\n",
        &[],
    );
}

#[test]
fn escaping_reference_diagnostic_labels_return_and_local_storage() {
    let source =
        "Int is in 0 to 100\nbad(input ref Int) ref Int:\n    value Int: 7\nreturn ref value\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let flaws = typed.all_flaws();
    let (_, diagnostic) = flaws
        .iter()
        .find(|(_, d)| d.code.slug() == "type-reference-return-local-target")
        .expect("escaping reference diagnostic");
    assert_eq!(
        &source[diagnostic.span.start()..diagnostic.span.end()],
        "ref value"
    );
    assert_eq!(diagnostic.related.len(), 1);
    let declaration = &diagnostic.related[0];
    assert!(source[declaration.span.start()..declaration.span.end()].contains("value"));
    assert!(declaration.span.end() < diagnostic.span.start());
    assert!(diagnostic.help.as_deref().unwrap().contains("ownership"));
    diagnostic.print(source, "example.gin");
}
