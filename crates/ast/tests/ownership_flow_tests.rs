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
fn test_consume_via_tilde_no_false_positive() {
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
fn test_use_after_consume_via_tilde_detected() {
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
fn test_auto_thread_keeps_copyable_alive() {
    // Copyable (Int) via bare param — stays alive.
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
    assert!(!has_moved, "auto-thread copyable: {:?}", diags);
}

#[test]
fn test_auto_thread_chained_copyable() {
    // Chained bare calls on copyable type — stays alive.
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
    assert!(!has_moved, "chained thread copyable: {:?}", diags);
}

#[test]
fn test_auto_thread_then_consume_copyable() {
    // Bare call then consume — no diagnostic.
    let src = "\
read(x Int) Int: x
drop(eat y Int) Int: 0
main:
    val: 42
    read(val)
    drop(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_moved = diags
        .iter()
        .any(|d| d.code.slug() == "type-use-of-moved-value");
    assert!(!has_moved, "thread then consume copyable: {:?}", diags);
}

#[test]
fn test_auto_thread_mixed_consume_and_bare() {
    // Mix of bare and consumed params.
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
    assert!(!has_moved, "mixed tilde and bare: {:?}", diags);
}

#[test]
fn test_tilde_param_returned_is_error() {
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
fn test_consume_arg_on_bare_param_is_error() {
    // `eat` at call site on a bare (Threaded) param is a contract mismatch.
    let src = "\
read(x Int) Int: x
main:
    val: 42
    read(eat val)
return 0
";
    let diags = collect_ownership_diagnostics(src);
    let has_consume_on_bare = diags
        .iter()
        .any(|d| d.code.slug() == "type-consume-arg-on-bare-param");
    assert!(
        has_consume_on_bare,
        "expected ConsumeArgOnBareParam, got: {:?}",
        diags
    );
}

#[test]
fn test_tilde_param_unused_is_valid() {
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
fn test_bare_param_is_inferred() {
    // Parser-level: bare params have no explicit convention stored.
    let src = "print(s String):\nreturn 0\n";
    let ast = parser::cursor::TokenCursor::parse_source(src);
    let bind = ast.defs.get(&Intern::from_ref("print")).unwrap();
    assert!(bind.param_conventions.get(&Intern::from_ref("s")).is_none());
}
