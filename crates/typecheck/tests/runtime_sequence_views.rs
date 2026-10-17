mod support;

use ast::ConstValue;
use internment::Intern;
use support::{check_no_flaws_with, check_with, expect_flaw_code, flaw_codes};

const VIEW: &str = r#"
Byte is in 0 to 255
Byte has
    finish(self):
        return
replace(previous Byte, next Byte) Byte: return next
read(value ref Byte) Byte: return deref value
Length is in 0 to 255
Length has
    finish(self):
        return
View(element) has
    storage ref [element; length]
    length Length
    finish(self):
        return
Owner has
    bytes [Byte; 4]
    finish(self):
        return
prefix(source ref [Byte; 4], length Length) View(Byte):
    data: ref source.[0; length]
    return View(storage: data, length: length)
peek(view ref View(Byte), index is in 0 to 3) Byte: return view.storage.[index]
"#;

#[test]
fn runtime_view_traversal_yields_observing_element_references() {
    let typed = check_no_flaws_with(
        VIEW,
        "visit(view ref View(Byte)):\n    for value in view.storage\n        read(value)\n    return\n",
    );
    let binding = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| match kind {
            typecheck::TypedExprKind::Bind { name, .. } if name.as_str() == "value" => Some(index),
            _ => None,
        })
        .unwrap();
    assert!(
        typecheck::ty::reference_pointee_for_type(
            &typed.exprs.ty[binding],
            Some(&typed.type_registry)
        )
        .is_some()
    );
}

#[test]
fn view_cannot_escape_local_backing_array() {
    let typed = check_with(
        VIEW,
        "escape View(Byte):\n    source [Byte; 4]: [10, 20, 30, 40]\n    return prefix(source, 2)\n",
    );
    expect_flaw_code(&typed, "type-reference-return-local-target");
}

#[test]
fn indexed_view_read_detects_consumed_storage() {
    let typed = check_with(
        VIEW,
        "bad(owner Owner) Byte:\n    view: prefix(owner.bytes, 2)\n    owner.finish\n    return peek(view, 1)\n",
    );
    expect_flaw_code(&typed, "type-use-of-invalidated-reference");
}

#[test]
fn indexed_array_field_read_detects_consumed_storage() {
    let typed = check_with(
        VIEW,
        "ArrayBorrow has\n    storage ref [Byte; 4]\n    finish(self):\n        return\nread_array(view ref ArrayBorrow, index is in 0 to 3) Byte: return view.storage.[index]\nbad(owner Owner) Byte:\n    view: ArrayBorrow(storage: owner.bytes)\n    owner.finish\n    return read_array(view, 1)\n",
    );
    expect_flaw_code(&typed, "type-use-of-invalidated-reference");
}

#[test]
fn view_constructor_rejects_a_constant_oversized_extent() {
    let typed = check_with(
        VIEW,
        "bad(source ref [Byte; 4]) View(Byte): return View(storage: source, length: 5)\n",
    );
    expect_flaw_code(&typed, "type-dependent-array-length");
}

#[test]
fn view_loop_cannot_consume_borrowed_elements() {
    let typed = check_with(
        VIEW,
        "bad(view ref View(Byte)):\n    for value in view.storage\n        value.finish\n    return\n",
    );
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn compile_time_view_iteration_uses_the_visible_length() {
    let typed = check_no_flaws_with(
        VIEW,
        "run(length Length) Byte:\n    source [Byte; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    total Byte: 0\n    for value in view.storage\n        previous is in Byte: total\n        amount is in Byte: read(value)\n        total:: replace(total, (previous + amount) as Byte)\n    return total\n",
    );
    let target = flask::CompileTarget::Library;
    for (length, expected) in [(0, 0), (1, 10), (2, 30), (4, 100)] {
        let result = typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        )
        .call(
            typecheck::DefId::named(Intern::from_ref("run")),
            &[ConstValue::Int(length.into())],
        )
        .unwrap();
        assert_eq!(result, ConstValue::Int(expected.into()));
    }
}

#[test]
fn compile_time_view_index_checks_the_visible_length() {
    let typed = check_no_flaws_with(
        VIEW,
        "run(length Length) Byte:\n    source [Byte; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    return peek(view, 3)\n",
    );
    let target = flask::CompileTarget::Library;
    let result = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    )
    .call(
        typecheck::DefId::named(Intern::from_ref("run")),
        &[ConstValue::Int(2.into())],
    )
    .unwrap_err();
    assert_eq!(
        result.code.slug().as_str(),
        "compile-time-index-out-of-bounds"
    );
}

#[test]
fn observing_view_fields_and_elements_cannot_be_mutated() {
    for body in [
        "view.length:: replace_length(view.length, 5)",
        "view.storage.[0]:: replace(view.storage.[0], 10)",
        "mut selected Length: view.length",
    ] {
        let typed = check_with(
            VIEW,
            &format!(
                "replace_length(previous Length, next Length) Length: return next\nbad(view mut View(Byte)):\n    {body}\n    return\n"
            ),
        );
        assert!(
            !typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn traversing_a_view_borrows_its_binding_and_checks_storage_reads() {
    check_no_flaws_with(
        VIEW,
        "visit(view View(Byte)):\n    for value in view.storage\n        read(value)\n    view.finish\n    return\n",
    );
    let typed = check_with(
        VIEW,
        "bad(owner Owner):\n    view: prefix(owner.bytes, 2)\n    for value in view.storage\n        owner.finish\n        read(value)\n    return\n",
    );
    expect_flaw_code(&typed, "type-use-of-invalidated-reference");
}

const MUTABLE_VIEW: &str = r#"
MutableView(element) has
    storage mut [element; length]
    length Length
    finish(self):
        return
mutable_prefix(source mut [Byte; 4], length Length) MutableView(Byte):
    data: mut source.[0; length]
    return MutableView(storage: data, length: length)
set(value mut Byte):
    value:: replace(deref value, 42)
    return
"#;

#[test]
fn mutable_views_yield_mutable_elements_and_allow_indexed_writes() {
    let typed = check_no_flaws_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}visit(view mut MutableView(Byte)):\n    view.storage.[0]:: replace(view.storage.[0], 10)\n    for value in view.storage\n        set(value)\n    return\n"
        ),
    );
    let ty = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| {
            matches!(kind, typecheck::TypedExprKind::Bind { name, .. } if name.as_str() == "value")
                .then_some(&typed.exprs.ty[index])
        })
        .unwrap();
    assert!(ty.is_mutable_ref());
}

#[test]
fn observing_a_mutable_view_does_not_grant_mutation() {
    for body in [
        "view.storage.[0]:: replace(view.storage.[0], 10)",
        "for value in view.storage\n        set(value)\n",
    ] {
        let typed = check_with(
            VIEW,
            &format!("{MUTABLE_VIEW}bad(view ref MutableView(Byte)):\n    {body}\n    return\n"),
        );
        assert!(!typed.all_flaws().is_empty(), "{body}");
    }
}

#[test]
fn mutable_view_metadata_stays_immutable() {
    for body in [
        "view.length:: replace_length(view.length, 1)",
        "mut selected Length: view.length",
    ] {
        let typed = check_with(
            VIEW,
            &format!(
                "{MUTABLE_VIEW}replace_length(previous Length, next Length) Length: return next\nbad(view mut MutableView(Byte)):\n    {body}\n    return\n"
            ),
        );
        assert!(
            !typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn mutable_view_cannot_escape_local_storage() {
    let typed = check_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}escape MutableView(Byte):\n    source [Byte; 4]: [1, 2, 3, 4]\n    return mutable_prefix(source, 2)\n"
        ),
    );
    expect_flaw_code(&typed, "type-reference-return-local-target");
}

#[test]
fn mutable_traversal_rejects_access_through_an_overlapping_alias() {
    for body in [
        "read(source.[0])",
        "source.[0]:: replace(source.[0], 5)",
        "read(alias.storage.[0])",
        "alias.storage.[0]:: replace(alias.storage.[0], 5)",
        "for inner in view.storage\n            set(inner)\n",
    ] {
        let typed = check_with(
            VIEW,
            &format!(
                "{MUTABLE_VIEW}bad(source mut [Byte; 4]):\n    view: mutable_prefix(source, 2)\n    alias: mutable_prefix(source, 2)\n    for value in view.storage\n        {body}\n        set(value)\n    alias.finish\n    view.finish\n    return\n"
            ),
        );
        assert!(
            flaw_codes(&typed).contains(&"type-sequence-borrow-conflict".to_owned()),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn mutable_traversal_allows_disjoint_storage_and_access_after_the_loop() {
    check_no_flaws_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}visit(left mut [Byte; 4], right mut [Byte; 4]):\n    view: mutable_prefix(left, 2)\n    for value in view.storage\n        read(right.[0])\n        set(value)\n    view.finish\n    read(left.[0])\n    return\n"
        ),
    );
}

#[test]
fn mutable_view_constructor_cannot_upgrade_shared_storage() {
    let typed = check_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}bad(source ref [Byte; 4]) MutableView(Byte): return MutableView(storage: source, length: 2)\n"
        ),
    );
    expect_flaw_code(&typed, "type-mutable-borrow-from-shared");
}

#[test]
fn compile_time_mutable_views_write_the_backing_storage() {
    let typed = check_no_flaws_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}run(length Length) Byte:\n    source [Byte; 4]: [10, 20, 30, 40]\n    view: mutable_prefix(source, length)\n    for value in view.storage\n        set(value)\n    view.finish\n    return source.[0]\n"
        ),
    );
    let target = flask::CompileTarget::Library;
    for (length, expected) in [(0, 10), (1, 42), (4, 42)] {
        let result = typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        )
        .call(
            typecheck::DefId::named(Intern::from_ref("run")),
            &[ConstValue::Int(length.into())],
        )
        .unwrap();
        assert_eq!(result, ConstValue::Int(expected.into()));
    }
}

#[test]
fn mutable_traversal_keeps_sibling_fields_disjoint() {
    let setup = format!(
        "{MUTABLE_VIEW}Pair has\n    left [Byte; 4]\n    right [Byte; 4]\n    finish(self):\n        return\n"
    );
    check_no_flaws_with(
        VIEW,
        &format!(
            "{setup}visit(pair mut Pair):\n    view: mutable_prefix(pair.left, 2)\n    for value in view.storage\n        read(pair.right.[0])\n        set(value)\n    view.finish\n    return\n"
        ),
    );
    let typed = check_with(
        VIEW,
        &format!(
            "{setup}bad(pair mut Pair):\n    view: mutable_prefix(pair.left, 2)\n    for value in view.storage\n        read(pair.left.[0])\n        set(value)\n    view.finish\n    return\n"
        ),
    );
    expect_flaw_code(&typed, "type-sequence-borrow-conflict");
}

#[test]
fn mutable_traversal_cannot_consume_elements() {
    let typed = check_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}bad(view mut MutableView(Byte)):\n    for value in view.storage\n        value.finish\n    return\n"
        ),
    );
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn nested_mutable_traversals_require_disjoint_storage() {
    check_no_flaws_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}visit(left mut [Byte; 4], right mut [Byte; 4]):\n    outer: mutable_prefix(left, 2)\n    inner: mutable_prefix(right, 2)\n    for first in outer.storage\n        for second in inner.storage\n            set(second)\n        set(first)\n    outer.finish\n    inner.finish\n    return\n"
        ),
    );
}

#[test]
fn compile_time_view_construction_preserves_complete_fields() {
    let typed = check_no_flaws_with(
        VIEW,
        "run(length Length) Byte:\n    source [Byte; 4]: [10, 20, 30, 40]\n    view: prefix(source, length)\n    return peek(view, 0)\n",
    );
    let target = flask::CompileTarget::Library;
    assert_eq!(
        typecheck::eval::Evaluator::new(
            typed.resolved_program(),
            typecheck::eval::CompileTimeHost::new(&target),
        )
        .call(
            typecheck::DefId::named(Intern::from_ref("run")),
            &[ConstValue::ONE]
        ),
        Ok(ConstValue::Int(10.into()))
    );
}

#[test]
fn replacing_an_element_invalidates_its_borrow_and_preserves_the_array_view() {
    check_no_flaws_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}visit(source mut [Byte; 4]) Byte:\n    view: mutable_prefix(source, 2)\n    source.[0]:: replace(source.[0], 5)\n    result: read(view.storage.[0])\n    view.finish\n    return result\n"
        ),
    );
    let typed = check_with(
        VIEW,
        &format!(
            "{MUTABLE_VIEW}bad(source mut [Byte; 4]) Byte:\n    ref selected Byte: source.[0]\n    source.[0]:: replace(source.[0], 5)\n    return read(selected)\n"
        ),
    );
    expect_flaw_code(&typed, "type-use-of-invalidated-reference");
}
