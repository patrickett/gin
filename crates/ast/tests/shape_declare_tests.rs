//! Tests for shape declarations, trait implementations, unions, and methods.
//!
//! Layer 7: shape declarations (`has` bodies)
//! Layer 8: trait provisions (`Type.Trait has ...`
//! Layer 9: union declarations with payload variants
//! Layer 10: namespaced and receiver methods
//!
//! Each test documents current behaviour — some assertions record expected
//! diagnostics that may become unnecessary as the compiler matures.

use ast::ty::Ty;
use internment::Intern;
use typecheck::{DefId, TagId};

mod support;
use support::transform_source;

#[test]
fn simple_shape_with_fields() {
    let src = "Int is in 1...400\n\nCoord has x Int, y Int, z Int";
    let typed = transform_source(src);

    let coord_id = TagId(Intern::new("Coord".to_string()));
    let tag = typed.tags.get(&coord_id).expect("Coord tag exists");

    match &tag.resolved_ty {
        Ty::Record { name, fields, .. } => {
            assert_eq!(name.as_str(), "Coord", "record name");
            assert_eq!(fields.len(), 3, "three fields");

            let field_names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(field_names, vec!["x", "y", "z"], "field names");

            // Each field type should be a resolved type (no UnknownSymbol flaws).
            for (fname, fty) in fields {
                assert!(
                    !matches!(fty.as_ref(), Ty::Opaque(n) if n.as_str() == "Unknown"),
                    "field {fname} should not be Unknown"
                );
            }
        }
        other => panic!("Expected Record type, got {other:?}"),
    }

    let flaws = typed.all_flaws();
    let unknown: Vec<_> = flaws
        .iter()
        .filter(|(_, f)| f.code.slug() == "type-unknown-symbol")
        .collect();
    assert!(unknown.is_empty(), "no unknown-symbol flaws: {unknown:?}");
}

/// 7.1b — Has members with methods don't become record fields.
#[test]
fn has_method_does_not_become_record_field() {
    let src =
        "Int is in 1...400\n\nRange(x) has start x, end x, contains(ref self, value x) Bool\n";
    let typed = transform_source(src);

    let range_id = TagId(Intern::new("Range".to_string()));
    let tag = typed.tags.get(&range_id).expect("Range tag exists");

    match &tag.resolved_ty {
        Ty::Record { name, fields, .. } => {
            assert_eq!(name.as_str(), "Range", "record name");
            assert_eq!(fields.len(), 2, "only 2 fields (not contains)");

            let field_names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(field_names, vec!["start", "end"], "field names");

            assert!(
                !field_names.contains(&"contains"),
                "contains should not be a record field"
            );
        }
        other => panic!("Expected Record type, got {other:?}"),
    }

    let flaws = typed.all_flaws();
    let unknown: Vec<_> = flaws
        .iter()
        .filter(|(_, f)| f.code.slug() == "type-unknown-symbol")
        .collect();
    assert!(unknown.is_empty(), "no unknown-symbol flaws: {unknown:?}");
}

/// 7.2 — Generic shape with type parameters.
#[test]
fn generic_shape_with_type_params() {
    let src = "Pair(a, b) has first a, second b";
    let typed = transform_source(src);

    let pair_id = TagId(Intern::new("Pair".to_string()));
    let tag = typed.tags.get(&pair_id).expect("Pair tag exists");

    // Generic parameters should be captured.
    if let Some(params) = &tag.params {
        assert!(
            params.contains_key(&Intern::new("a".to_string())),
            "param a"
        );
        assert!(
            params.contains_key(&Intern::new("b".to_string())),
            "param b"
        );
        assert_eq!(params.len(), 2, "two generic params");
    } else {
        panic!("expected generic params on Pair, got none");
    }

    match &tag.resolved_ty {
        Ty::Record { name, fields, .. } => {
            assert_eq!(name.as_str(), "Pair");
            assert_eq!(fields.len(), 2);

            // Field types should be the generic param references (Opaque).
            let first_ty = &fields[0].1;
            let second_ty = &fields[1].1;
            assert_eq!(fields[0].0.as_str(), "first");
            assert_eq!(fields[1].0.as_str(), "second");

            // The fields reference the generic types a and b as Opaque.
            assert!(
                matches!(first_ty.as_ref(), Ty::Opaque(n) if n.as_str() == "a"),
                "first field should be Opaque(a), got {first_ty:?}"
            );
            assert!(
                matches!(second_ty.as_ref(), Ty::Opaque(n) if n.as_str() == "b"),
                "second field should be Opaque(b), got {second_ty:?}"
            );
        }
        other => panic!("Expected Record type, got {other:?}"),
    }
}

/// 7.3 — Shape with method signature as member.
#[test]
fn shape_with_interface_member() {
    let src = "Iterator(item) has next Maybe(item)";
    let typed = transform_source(src);

    // Should not crash during transform.
    let iter_id = TagId(Intern::new("Iterator".to_string()));
    let tag = typed.tags.get(&iter_id).expect("Iterator tag exists");

    // The `resolved_ty` should be a Record.
    assert!(
        matches!(&tag.resolved_ty, Ty::Record { .. }),
        "Iterator should be a Record, got {:?}",
        tag.resolved_ty
    );

    // No crashes even if field types reference undefined tags.
    let flaws = typed.all_flaws();
    let _ = flaws; // document current behaviour
}

/// 7.4 — Shape with parameterised interface members (return type + error type).
#[test]
fn shape_with_parameterized_interface_members() {
    let src = "\
AllocError is Oom or InvalidLayout
Allocator has
    reserve(ref self, l Int) Slice(Byte) or AllocError
    release(ref self, p Int, l Int)
";
    let typed = transform_source(src);

    // Transform should not crash.
    let alloc_id = TagId(Intern::new("Allocator".to_string()));
    let tag = typed.tags.get(&alloc_id).expect("Allocator tag exists");

    assert!(
        matches!(&tag.resolved_ty, Ty::Record { .. }),
        "Allocator should be a Record, got {:?}",
        tag.resolved_ty
    );

    // Check the underlying AST value is Interface with two members.
    // (We can check via the declaration AST by re-parsing.)
    let flaws = typed.all_flaws();
    let _ = flaws;
}

#[test]
fn separate_trait_impl() {
    let src = "\
Int is in 1...400
Coord has x Int, y Int
Default has default Self
Coord.Default has default: Coord(x: 0, y: 0)";
    let typed = transform_source(src);

    let flaws = typed.all_flaws();
    // The trait implementation may or may not produce unknown-symbol flaws
    // depending on whether Default/Reflectable/etc. are in scope.
    // Currently this documents that the transform doesn't crash.
    assert!(
        typed
            .tags
            .contains_key(&TagId(Intern::new("Coord".to_string()))),
        "Coord tag should exist"
    );
    let _ = flaws;
}

/// 8.2 — Trait impl using dot syntax.
///
/// The dot implementation declaration adds a `ProvidedTrait`; the compiler also
/// synthesises `Reflectable` for every user-declared tag.
#[test]
fn trait_impl_with_dot_syntax() {
    let src = "\
Int is in 1...400
Coord has x Int, y Int
Coord.Default has default: Coord(x: 0, y: 0)";
    let typed = transform_source(src);

    let coord_id = TagId(Intern::new("Coord".to_string()));
    let tag = typed.tags.get(&coord_id).expect("Coord tag exists");

    // The dot implementation declaration should produce a ProvidedTrait entry.
    let has_default = tag
        .provided_traits
        .iter()
        .any(|pt| pt.trait_name.as_str() == "Default");
    assert!(
        has_default,
        "should have a ProvidedTrait for Default, got: {:?}",
        tag.provided_traits
    );

    // Compiler always synthesises Reflectable.
    let has_reflectable = tag
        .provided_traits
        .iter()
        .any(|pt| pt.trait_name.as_str() == "Reflectable");
    assert!(has_reflectable, "Reflectable should be synthesised");

    let flaws = typed.all_flaws();
    let _ = flaws;
}

#[test]
fn explicit_copy_provision() {
    let src = "\
Int is in 1...400
Copy has can_copy Bool
UniqueId has id Int
UniqueId.Copy has can_copy: False";
    let typed = transform_source(src);

    let uid_id = TagId(Intern::new("UniqueId".to_string()));
    let tag = typed.tags.get(&uid_id).expect("UniqueId tag exists");

    let has_copy = tag
        .provided_traits
        .iter()
        .any(|pt| pt.trait_name.as_str() == "Copy");
    assert!(
        has_copy,
        "UniqueId should have an explicit Copy override, got: {:?}",
        tag.provided_traits
    );

    let flaws = typed.all_flaws();
    let _ = flaws;
}

#[test]
fn union_with_named_variant_fields() {
    // `Int` is declared so it resolves; `String` is an undeclared tag
    // so it will produce an unknown-symbol flaw.
    let src = "\
Int is in 1...400
Status is
    Pending(eta Int, message String) or
    Complete";
    let typed = transform_source(src);

    let status_id = TagId(Intern::new("Status".to_string()));
    let tag = typed.tags.get(&status_id).expect("Status tag exists");

    match &tag.resolved_ty {
        Ty::Union { name, variants, .. } => {
            assert_eq!(name.as_str(), "Status");

            // Pending should have named fields.
            let pending = variants
                .iter()
                .find(|v| v.name.as_str() == "Pending")
                .expect("Pending variant");
            assert_eq!(pending.fields.len(), 2, "Pending has two fields");
            assert_eq!(pending.fields[0].0.as_str(), "eta", "first field name");
            assert_eq!(pending.fields[1].0.as_str(), "message", "second field name");

            // Complete should have no fields.
            let complete = variants
                .iter()
                .find(|v| v.name.as_str() == "Complete")
                .expect("Complete variant");
            assert!(complete.fields.is_empty(), "Complete has no fields");
        }
        other => panic!("Expected Union type, got {other:?}"),
    }

    let flaws = typed.all_flaws();
    // `String` may produce an unknown-symbol flaw, but `Int` should not.
    let string_unknown = flaws
        .iter()
        .any(|(_, f)| f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("String"));
    let int_unknown = flaws
        .iter()
        .any(|(_, f)| f.code.slug() == "type-unknown-symbol" && f.arg("name") == Some("Int"));
    assert!(
        !int_unknown,
        "Int should not be unknown (declared in same file)"
    );
    // `String` being unknown is expected since it's not in scope.
    let _ = string_unknown;
}

/// 9.2 — Union with generic variant payload field.
#[test]
fn union_with_generic_variant() {
    let src = "Maybe(value) is Some(value) or None";
    let typed = transform_source(src);

    let maybe_id = TagId(Intern::new("Maybe".to_string()));
    let tag = typed.tags.get(&maybe_id).expect("Maybe tag exists");

    match &tag.resolved_ty {
        Ty::Union { name, variants, .. } => {
            assert_eq!(name.as_str(), "Maybe");

            let some = variants
                .iter()
                .find(|v| v.name.as_str() == "Some")
                .expect("Some variant");
            assert_eq!(some.fields.len(), 1, "Some has one field");
            assert_eq!(some.fields[0].0.as_str(), "value", "field name");

            let none = variants
                .iter()
                .find(|v| v.name.as_str() == "None")
                .expect("None variant");
            assert!(none.fields.is_empty(), "None has no fields");
        }
        other => panic!("Expected Union type, got {other:?}"),
    }
}

/// 9.3 — Variant construction with named args.
#[test]
fn variant_construction_with_named_args() {
    let src = "\
Int is in 1...400
Status is Pending(eta Int, message String) or Complete
s := Status.Pending(eta: 1, message: \"starting\")
d := Status.Complete";
    let typed = transform_source(src);

    // Check that both defs exist.
    let s_id = DefId(Intern::new("s".to_string()));
    let d_id = DefId(Intern::new("d".to_string()));

    assert!(typed.defs.contains_key(&s_id), "def s should exist");
    assert!(typed.defs.contains_key(&d_id), "def d should exist");

    let _flaws = typed.all_flaws();
}

#[test]
fn namespaced_method() {
    let src = "\
Int is in 1...400
Coord has x Int, y Int
Coord.new(x Int, y Int) Int: 0";
    let typed = transform_source(src);

    // `Coord.new` should be a valid function definition.
    let new_id = DefId(Intern::new("Coord.new".to_string()));
    assert!(
        typed.defs.contains_key(&new_id),
        "Coord.new should exist as a def"
    );
    let def = typed.defs.get(&new_id).expect("Coord.new");
    assert!(def.name.as_str().contains("Coord.new"));

    let flaws = typed.all_flaws();
    let _ = flaws;
}

/// 10.2 — Receiver method with explicit self type.
///
/// The method `Coord.length(self Int)` uses `self` as the receiver.
/// The compiler currently does not populate `receiver_type` on
/// `TypedBind` (it remains `None` during declare stage), but the
/// parser correctly recognises the method syntax.
#[test]
fn receiver_method_with_explicit_self() {
    let src = "\
Int is in 1...400
Coord has x Int, y Int
Coord.length(self Int) Int: self";
    let typed = transform_source(src);

    let len_id = DefId(Intern::new("Coord.length".to_string()));
    assert!(
        typed.defs.contains_key(&len_id),
        "Coord.length should exist"
    );

    // Transform should not crash; receiver_type remains None on TypedBind.
    let flaws = typed.all_flaws();
    let has_self_param_typed = flaws
        .iter()
        .any(|(_, f)| f.code.slug() == "type-self-param-typed");
    // Explicit self type may warn, which is current documented behaviour.
    let _ = has_self_param_typed;
    let _ = flaws;
}

/// 10.3 — Unit return method.
#[test]
fn unit_return_method() {
    let src = "\
Int is in 1...400
Coord has x Int, y Int
Coord.reset(self Int):
    self + 0";
    let typed = transform_source(src);

    let reset_id = DefId(Intern::new("Coord.reset".to_string()));
    assert!(
        typed.defs.contains_key(&reset_id),
        "Coord.reset should exist"
    );

    // Transform should not crash; receiver_type remains None on TypedBind.
    let flaws = typed.all_flaws();
    let _ = flaws;
}
