mod support;

use ast::ConstValue;
use internment::Intern;
use support::{check_no_flaws, expect_flaw_code_named};
use typecheck::{BindBody, DefId, TypedExprKind, TypedFileAst};

fn outer_return(typed: &TypedFileAst) -> typecheck::ExprId {
    let BindBody::Body { ret: Some(ret), .. } =
        typed.defs[&DefId::named(Intern::from_ref("outer"))].body
    else {
        panic!("outer return")
    };
    ret
}

#[test]
fn declaring_a_callable_does_not_export_its_local_constants() {
    let typed = check_no_flaws(
        "outer():\n    x := 0\n    local():\n        value := 7\n        return value\n    return x\n",
    );
    let local_body = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            TypedExprKind::Bind { name, body, .. } if *name == Intern::from_ref("local") => {
                Some(*body)
            }
            _ => None,
        })
        .expect("local callable body");
    assert_eq!(
        typed.exprs.const_value_of(local_body),
        Some(&Some(ConstValue::Int(7_i128.into())))
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn declaring_a_callable_does_not_apply_its_capture_assignment() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    local():\n        x :: 7\n        return x\n    return x\n",
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_callable_invalidates_constants_for_mutated_captures() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    local():\n        x :: 7\n        return x\n    local()\n    return x\n",
    );
    assert_ne!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_pure_callable_preserves_capture_constants() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    local(): return x\n    local()\n    return x\n",
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_callable_preserves_constants_for_unaffected_places() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    y is in 0 to 255: 3\n    local():\n        y :: 7\n        return y\n    local()\n    return x\n",
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_callable_tracks_writes_through_nested_local_calls() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    local():\n        nested():\n            x :: 7\n            return x\n        nested()\n        return 0\n    local()\n    return x\n",
    );
    assert_ne!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_callable_with_a_mutating_external_call_discards_capture_facts() {
    let typed = check_no_flaws(
        "write(value mut is in 0 to 255):\n    value:: 7\n    return\nouter():\n    x is in 0 to 255: 0\n    local():\n        write(x)\n        return 0\n    local()\n    return x\n",
    );
    assert_ne!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn recursive_pure_local_calls_terminate_write_analysis_and_preserve_constants() {
    let typed = check_no_flaws(
        "Bit is in 0 to 1\nBit has finish(self): return\nouter():\n    x is in 0 to 255: 0\n    local(value Bit) Bit:\n        when value is in 0 to 0 then\n            return 0\n        return local(0)\n    local(1)\n    return x\n",
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn invoking_a_callable_does_not_apply_uncalled_nested_writes() {
    let typed = check_no_flaws(
        "outer():\n    x is in 0 to 255: 0\n    local():\n        nested():\n            x :: 7\n            return x\n        return 0\n    local()\n    return x\n",
    );
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn nested_callable_scopes_restore_their_parent_constants() {
    let typed = check_no_flaws(
        "outer():\n    x := 0\n    local():\n        local_value := 3\n        nested():\n            nested_value := 7\n            return nested_value\n        return local_value\n    return x\n",
    );
    for (name, expected) in [("local", 3_i128), ("nested", 7)] {
        let body = typed
            .exprs
            .kind
            .iter()
            .find_map(|kind| match kind {
                TypedExprKind::Bind {
                    name: actual, body, ..
                } if *actual == Intern::from_ref(name) => Some(*body),
                _ => None,
            })
            .expect("callable body");
        assert_eq!(
            typed.exprs.const_value_of(body),
            Some(&Some(ConstValue::Int(expected.into()))),
            "{name}"
        );
    }
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(0_i128.into())))
    );
}

#[test]
fn ordinary_local_initializers_still_propagate_constants() {
    let typed = check_no_flaws("outer():\n    x := 0\n    result: 7\n    return result\n");
    assert_eq!(
        typed.exprs.const_value_of(outer_return(&typed)),
        Some(&Some(ConstValue::Int(7_i128.into())))
    );
}

const CASES: &[(&str, &str, &str)] = &[
    (
        "local_capture_writes_invalidate_return_refinement_proofs",
        "outer() is in 0 to 1:\n    x is in 0 to 255: 0\n    local():\n        x :: 7\n        return x\n    local()\n    return x\n",
        "type-integer-narrowing-failed",
    ),
    (
        "isolated_callable_bodies_still_report_invalid_reassignments",
        "outer():\n    local():\n        x := 7\n        x :: 8\n        return x\n    return 0\n",
        "type-rebind-immutable",
    ),
];

#[test]
fn local_callable_flow_cases() {
    for &(name, source, code) in CASES {
        expect_flaw_code_named(source, code, name);
    }
}
