//! Flow analysis tests for the ownership system.
//!
//! Covers the core rules of the linear value ownership model:
//!   1. Creation   — a linear value is tracked by the type system
//!   2. Transfer   — ownership moves when passed to a `eat` param
//!   3. Consumption— using a linear value fulfills the "use once" requirement
//!   4. No copying — implicit duplication is rejected (size-based: ≤ 16 bytes = Copy)
//!   5. No dropping— implicit discard is rejected (> 16 bytes = linear)
//!   6. Borrowing  — non-owning references (TakePtr/Deref) preserve ownership
//!   7. Aliasing   — aliases only under single-ownership-preserving rules
//!   8. Return     — linear value must be consumed or transferred before scope exit
//!
//! Tests are grouped by the diagnostic they expect.
//!
//! NOTE: gin_core modules are NOT loaded in these tests. User-defined types
//! resolve to Ty::Opaque (8 bytes, ≤ 16, Copy) when they can't be found.
//! LinValueNotConsumed flow tests are therefore limited — the copyability unit
//! tests in `test_copy_inference_*` directly verify the size-based rule.

use internment::Intern;
use typecheck::FileId;
use typecheck::analysis::TyCopyExt;
use typecheck::transform::transform_file;

/// `Int` tag so ownership tests can type parameters without loading gin_core.
const INT_TAG: &str = "Int is in 1...400\n\n";

fn ownership_source(body: &str) -> String {
    format!("{INT_TAG}{body}")
}

/// Parse source, transform via typed AST pipeline, and collect diagnostics.
fn collect_ownership_diagnostics(source: &str) -> Vec<diagnostic::Diagnostic> {
    let ast = parser::cursor::TokenCursor::parse_source(&ownership_source(source));
    let typed = transform_file(ast.clone(), FileId(0));
    let mut diags = Vec::new();
    for (_, flaw) in typed.all_flaws() {
        diags.push(flaw.clone());
    }
    diags
}

#[test]
fn test_use_after_move_produces_diagnostic() {
    // Consume via `eat` at call site — subsequent use should produce UseOfMovedValue.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    result: val
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved_diag = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        has_moved_diag,
        "expected UseOfMovedValue diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_double_consume_produces_diagnostic() {
    // Two consecutive `eat` on the same value — second is UseOfMovedValue.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        has_moved,
        "expected UseOfMovedValue for double consume, got: {:?}",
        diags
    );
}

#[test]
fn test_no_false_positive_use_after_move_copyable() {
    // Copyable (Int) value used normally — no UseOfMovedValue.
    let src = "\
main:
    val: 42
    result: val + 1
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved_diag = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved_diag, "unexpected UseOfMovedValue diagnostic");
}

#[test]
fn test_consume_via_eat_no_false_positive() {
    // `eat` at call site matched with `eat` param — no diagnostic.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "unexpected UseOfMovedValue: {:?}", diags);
}

#[test]
fn test_use_after_consume_via_eat_detected() {
    // Consume via `eat`, then use — should be an error.
    let src = "\
drop(eat x Int) Int: 0
main:
    val: 42
    drop(eat val)
    result: val
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(has_moved, "use after eat consume: {:?}", diags);
}

#[test]
fn test_default_owned_copy_argument_remains_alive() {
    // Passing a structurally Copy value by ownership copies it.
    let src = "\
read(x Int) Int: x
main:
    val: 42
    read(val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "owned Copy argument moved: {:?}", diags);
}

#[test]
fn test_default_owned_non_copy_argument_moves() {
    let src = "\
sink(pointer Pointer(Int)): eat pointer
main:
    value: 42
    pointer: @value
    sink(pointer)
    result: deref pointer
return 0
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-moved-value"),
        "expected owned non-Copy argument to move, got: {:?}",
        diags
    );
}

#[test]
fn test_default_owned_copy_arguments_can_be_chained() {
    // Each ownership pass copies a structurally Copy value.
    let src = "\
add_one(x Int) Int: x + 1
double(y Int) Int: y * 2
main:
    val: 42
    s1: add_one(val)
    r: double(s1)
return r
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(
        !has_moved,
        "chained owned Copy arguments moved: {:?}",
        diags
    );
}

#[test]
fn test_move_in_when_arm_does_not_poison_sibling_arm() {
    let src = "\
sink(pointer Pointer(Int)) Int: eat pointer
select(flag Int, pointer Pointer(Int)) Int:
    when flag is
        0 then sink(pointer)
        else deref pointer
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        !diags.iter().any(|diagnostic| matches!(
            diagnostic.code.slug(),
            "type-use-of-moved-value" | "type-use-of-maybe-moved-value"
        )),
        "exclusive sibling arm saw moved value: {:?}",
        diags
    );
}

#[test]
fn test_move_on_one_when_path_is_maybe_moved_after_join() {
    let src = "\
sink(pointer Pointer(Int)) Int: eat pointer
select(flag Int, pointer Pointer(Int)) Int:
    when flag is
        0 then sink(pointer)
        else 0
    result: deref pointer
return result
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-maybe-moved-value"),
        "expected maybe-moved diagnostic after branch join, got: {:?}",
        diags
    );
}

#[test]
fn test_loop_backedge_rejects_reusing_moved_value() {
    let src = "\
sink(pointer Pointer(Int)): eat pointer
repeat(pointer Pointer(Int)):
    while 1
        sink(pointer)
    loop
return
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-moved-value"),
        "expected move error on a possible later iteration, got: {:?}",
        diags
    );
}

#[test]
fn test_reference_invalidated_on_one_when_path_is_maybe_invalid_after_join() {
    let src = "\
Cell has value Int
sink(cell Cell): eat cell
check(flag Int, cell Cell) Int:
    ref saved Cell: cell
    when flag is
        0 then sink(cell)
        else saved.value
    result: saved.value
return result
";
    let diags = collect_ownership_diagnostics(src);

    assert!(
        diags.iter().any(|diagnostic| {
            diagnostic.code.slug() == "type-use-of-maybe-invalidated-reference"
        }),
        "expected maybe-invalidated diagnostic after branch join, got: {:?}",
        diags
    );
    assert!(
        !diags
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "type-use-of-invalidated-reference"),
        "exclusive sibling arm saw invalidated reference: {:?}",
        diags
    );
}

#[test]
fn test_observe_then_consume() {
    let src = "\
read(ref x Int) Int: x
drop(eat y Int) Int: 0
main:
    val: 42
    read(ref val)
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "observe then consume: {:?}", diags);
}

#[test]
fn test_default_owned_copy_and_eat_arguments_can_be_mixed() {
    // Mix a default-owned Copy parameter with an explicitly consumed parameter.
    let src = "\
pair(a Int, eat b Int) Int: a + b
main:
    x: 10
    y: 20
    pair(x, eat y)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "mixed eat and bare: {:?}", diags);
}

#[test]
fn test_eat_param_returned_is_error() {
    // Returning a `eat` param is invalid (it's auto-dropped at scope exit).
    let src = "\
consume(eat x Int) Int: x
main:
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_return_consumed = diags
        .iter()
        .any(|d| d.code.slug() == "type-return-consumed-param");
    assert!(
        has_return_consumed,
        "expected ReturnConsumedParam, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_argument_on_default_owned_param_is_error() {
    // `eat` does not match a default-owned parameter.
    let src = "\
read(x Int) Int: x
main:
    val: 42
    read(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_mode_mismatch = diags
        .iter()
        .any(|d| d.code.slug() == "type-consume-arg-on-owned-param");
    assert!(
        has_mode_mismatch,
        "expected consume-on-owned diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_param_unused_is_valid() {
    // `eat` param that is never used in the body — auto-dropped, no diagnostic.
    let src = "\
leaky(eat x Int) Int: 0
main:
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_lin = diags
        .iter()
        .any(|d| d.code.slug() == "type-lin-value-not-consumed");
    assert!(
        !has_lin,
        "eat param unconsumed should be valid (auto-dropped), got: {:?}",
        diags
    );
}

#[test]
fn test_mut_call_preserves_existing_reference_without_invalidation_effect() {
    let src = "\
replace(mut x Int) Int:
    x: 0
return x
main:
    value: 42
    ref reference Int: value
    replace(mut value)
    result: reference
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_item_replacement_preserves_reference() {
    let src = "\
main:
    items: (0; 2)
    ref saved Int: items.(0)
    items.(0): 1
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_consuming_different_fixed_array_item_preserves_reference() {
    let src = "\
drop(eat value Int) Int: 0
main:
    items: (0; 2)
    ref saved Int: items.(0)
    drop(eat items.(1))
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_proves_index_refinement_from_length() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
main:
    items: (0; 4)
    ref saved Int: get(ref items, 2)
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags.iter().any(|d| {
            matches!(
                d.code.slug(),
                "type-parameter-refinement-failed" | "type-parameter-refinement-unproven"
            )
        }),
        "unexpected refinement diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_rejects_index_at_length() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
main:
    items: (0; 4)
    ref saved Int: get(ref items, 4)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-parameter-refinement-failed"),
        "expected failed refinement diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_fixed_array_call_requires_proof_for_symbolic_index() {
    let src = "\
get(ref array Array(x, n), index Int and >= 0 and < n) ref x: array.(index)
visit(ref items Array(Int, 4), index Int) Int:
    ref saved Int: get(ref items, index)
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-parameter-refinement-unproven"),
        "expected unproven refinement diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_parameter_refinement_proves_symbolic_array_items_disjoint() {
    let src = "\
drop(eat value Int) Int: 0
visit(ref items x, i Int, j Int and > i) Int:
    ref saved Int: items.(i)
    drop(eat items.(j))
    result: saved
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        !diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "unexpected invalidated reference diagnostic: {:?}",
        diags
    );
}

#[test]
fn test_unrelated_symbolic_array_items_conservatively_overlap() {
    let src = "\
drop(eat value Int) Int: 0
visit(ref items x, i Int, j Int) Int:
    ref saved Int: items.(i)
    drop(eat items.(j))
    result: saved
return 0
main: 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_consuming_same_fixed_array_item_invalidates_reference() {
    let src = "\
drop(eat value Int) Int: 0
main:
    items: (0; 2)
    ref saved Int: items.(0)
    drop(eat items.(0))
    result: saved
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

#[test]
fn test_eat_invalidates_existing_reference_on_later_use() {
    let src = "\
drop(eat x Int) Int: 0
main:
    value: 42
    ref reference Int: value
    drop(eat value)
    result: reference
return 0
";
    let diags = collect_ownership_diagnostics(src);
    assert!(
        diags
            .iter()
            .any(|d| d.code.slug() == "type-use-of-invalidated-reference"),
        "expected invalidated reference diagnostic, got: {:?}",
        diags
    );
}

mod support;
use support::{empty_typed, marker_trait_registry};

#[test]
fn test_copy_inference_int_is_copyable() {
    let int_ty = typecheck::ty::Ty::Int {
        width: 64,
        signed: true,
        value: None,
        min: None,
        max: None,
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    assert!(int_ty.is_copyable(&registry, &typed));
}

#[test]
fn test_copy_inference_ptr_is_not_copyable() {
    let ptr_ty = typecheck::ty::Ty::Ptr {
        inner: Box::new(typecheck::ty::Ty::Int {
            width: 64,
            signed: true,
            value: None,
            min: None,
            max: None,
        }),
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    assert!(!ptr_ty.is_copyable(&registry, &typed));
}

#[test]
fn test_copy_inference_small_record_is_copyable() {
    let small = typecheck::ty::Ty::Record {
        name: Intern::from_ref("Small"),
        resolved_params: None,
        fields: vec![(
            Intern::from_ref("x"),
            Box::new(typecheck::ty::Ty::Int {
                width: 64,
                signed: true,
                value: None,
                min: None,
                max: None,
            }),
        )],
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    assert!(small.is_copyable(&registry, &typed));
}

#[test]
fn test_copy_inference_record_with_copy_fields_is_copyable() {
    let large = typecheck::ty::Ty::Record {
        name: Intern::from_ref("Large"),
        resolved_params: None,
        fields: (0..5)
            .map(|i| {
                (
                    Intern::new(format!("f{i}")),
                    Box::new(typecheck::ty::Ty::Int {
                        width: 64,
                        signed: true,
                        value: None,
                        min: None,
                        max: None,
                    }),
                )
            })
            .collect(),
    };
    let registry = marker_trait_registry();
    let typed = empty_typed();
    assert!(large.is_copyable(&registry, &typed));
}

#[test]
fn test_bare_param_defaults_to_ownership() {
    let src = "print(s String):\nreturn 0\n";
    let ast = parser::cursor::TokenCursor::parse_source(src);
    let parsed = ast.defs.get(&Intern::from_ref("print")).unwrap();
    assert!(
        parsed
            .param_conventions
            .get(&Intern::from_ref("s"))
            .is_none()
    );

    let typed = transform_file(ast, FileId(0));
    let bind = typed
        .defs
        .get(&typecheck::DefId(Intern::from_ref("print")))
        .unwrap();
    assert_eq!(bind.param_conventions, vec![ast::ParamConvention::Own]);
}
