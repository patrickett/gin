mod support;

use parser::cursor::TokenCursor;
use support::{expect_flaws_lexed_with, expect_flaws_lexed_with_named, package};
use typecheck::FileId;
use typecheck::transform::{TransformCtx, transform};

const PRELUDE: &str = "Int is in 0 to 100\nInt has\n    finish(self):\n        return\n";

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "local_reference_without_input_group_is_rejected",
        "bad() ref Int:\n    value Int: 7\n    return ref value\n",
        &["type-reference-return-local-target"],
    ),
    (
        "local_reference_on_early_return_is_rejected",
        "bad(input ref Int, flag Int) ref Int:\n    value Int: 7\n    when flag is 1 then\n        return ref value\n    return input\n",
        &["type-reference-return-local-target"],
    ),
    (
        "wrong_group_on_early_return_is_rejected",
        "bad(left ref{a} Int, right ref{b} Int, flag Int) ref{a} Int:\n    when flag is 1 then\n        return right\n    return left\n",
        &["type-reference-return-wrong-target-group"],
    ),
    (
        "caller_reference_on_both_paths_is_allowed",
        "good(input ref Int, flag Int) ref Int:\n    when flag is 1 then\n        return input\n    return input\n",
        &[],
    ),
    (
        "owned_local_return_is_allowed",
        "good() Int:\n    value Int: 7\n    return value\n",
        &[],
    ),
    (
        "local_reference_in_named_results_is_rejected",
        "bad() value ref Int, number Int:\n    local Int: 7\n    return ref local, 0\n",
        &["type-reference-return-local-target"],
    ),
    (
        "caller_reference_through_local_alias_is_allowed",
        "good(input ref Int) ref Int:\n    ref saved Int: input\n    return saved\n",
        &[],
    ),
    (
        "nested_early_local_reference_is_rejected",
        "bad(input ref Int, flag Int) ref Int:\n    value Int: 7\n    when flag is 1 then\n        when flag is 1 then\n            return ref value\n        return input\n    return input\n",
        &["type-reference-return-local-target"],
    ),
    (
        "forwarded_named_reference_results_keep_their_groups",
        "pair(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return left, right\nforward(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return pair(left, right)\n",
        &[],
    ),
    (
        "forwarded_named_reference_results_reject_swapped_groups",
        "pair(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return left, right\nforward(left ref{a} Int, right ref{b} Int) first ref{a} Int, second ref{b} Int: return pair(right, left)\n",
        &["type-reference-return-wrong-target-group"],
    ),
    (
        "reference_to_pointer_projection_is_rejected",
        "bad(pointer @Int) ref Int: return ref pointer.[0]\n",
        &["type-reference-return-local-target"],
    ),
    (
        "local_reference_in_when_arm_is_rejected",
        "bad(input ref Int, flag Int) ref Int:\n    local Int: 7\n    return when flag is 1 then ref local else input\n",
        &["type-reference-return-local-target"],
    ),
    (
        "caller_reference_in_when_arms_is_allowed",
        "good(input ref Int, flag Int) ref Int: return when flag is 1 then input else input\n",
        &[],
    ),
    (
        "local_reference_returned_from_a_loop_is_rejected",
        "bad(input ref Int, flag Int) ref Int:\n    value Int: 7\n    while flag is 1 do\n        when flag is 1 then\n            return ref value\n    return input\n",
        &["type-reference-return-local-target"],
    ),
    (
        "forwarded_views_of_owned_storage_preserve_receiver_provenance",
        "View(element) has\n    length Int\n    address ref [element; length]\n    finish(self):\n        return\nStorage(element) has\n    #private\n    address own element\n    finish(self):\n        return\n    view(self ref) View(element):\n        data: ref self.address.[0; 0]\n        return View(length: 0, address: data)\nWrapper(element) has\n    storage Storage(element)\n    finish(self):\n        self.storage.finish\n        return\n    view(self ref) View(element): return self.storage.view\n",
        &[],
    ),
];

#[test]
fn reference_return_safety_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_lexed_with_named(PRELUDE, body, expected, name);
    }
}

#[test]
fn methods_can_borrow_their_receiver_and_its_fields() {
    for (mode, result, returned) in [
        ("ref", "Int", "inspect_value(self.value)"),
        ("own", "Packet", "self"),
    ] {
        expect_flaws_lexed_with(
            PRELUDE,
            &format!(
                "Packet has\n    value Int\n    finish(self):\n        return\n    inspect(self {mode}) {result}:\n        inspect_packet(self)\n        inspect_value(self.value)\n        return {returned}\n\ninspect_packet(packet ref Packet) Int: return packet.value\ninspect_value(value ref Int) Int: return deref value\n"
            ),
            &[],
        );
    }
}

#[test]
fn escaping_reference_diagnostic_labels_return_and_local_storage() {
    let source =
        "Int is in 0 to 100\nbad(input ref Int) ref Int:\n    value Int: 7\n    return ref value\n";
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
    assert_eq!(diagnostic.related().len(), 1);
    let declaration = &diagnostic.related()[0];
    assert!(source[declaration.span.start()..declaration.span.end()].contains("value"));
    assert!(declaration.span.end() < diagnostic.span.start());
    assert!(diagnostic.help.as_deref().unwrap().contains("ownership"));
    diagnostic.print(source, "example.gin");
}

#[test]
fn imported_views_of_owned_storage_preserve_receiver_provenance() {
    let output = package(
        "views",
        &[
            (
                "storage",
                "Int is in 0 to 100\nInt has\n    finish(self):\n        return\nView(element) has\n    length Int\n    address ref [element; length]\n    finish(self):\n        return\nStorage(element) has\n    #private\n    address own element\n    finish(self):\n        return\n    view(self ref) View(element):\n        data: ref self.address.[0; 0]\n        return View(length: 0, address: data)\n",
            ),
            (
                "wrapper",
                "use views.storage Storage, View\nWrapper(element) has\n    storage Storage(element)\n    finish(self):\n        self.storage.finish\n        return\n    view(self ref) View(element): return self.storage.view\n",
            ),
        ],
    );
    let flaws: Vec<_> = output.iter().flat_map(|typed| typed.all_flaws()).collect();
    assert!(flaws.is_empty(), "{flaws:?}");
}
