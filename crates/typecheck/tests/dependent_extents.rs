//! Dependent extents: `data [element; len]` with `len` a sibling field. An index is accepted
//! exactly when `0 <= index < len` is proved, and a view is built only from a slice whose length
//! is proved equal to `len`.
use typecheck::{
    CheckedSourceFile, CheckedSourceInstanceHead, Limits, SourceCheckError, SourceType, check_file,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict extent fixture");
    check_file(&file, Limits::default())
}

const BASE: &str = "\
#unrestricted
Byte is in 0 to 255
View has
  len in 0 to 16
  data ref [Byte; len]
MutView has
  len in 0 to 16
  data mut [Byte; len]
";

fn with(tail: &str) -> String {
    format!("{BASE}{tail}")
}

fn accepts(tail: &str) {
    let source = with(tail);
    assert!(check(&source).is_ok(), "{tail}: {:?}", check(&source));
}

fn rejects(tail: &str, error: SourceCheckError) {
    assert_eq!(check(&with(tail)), Err(error), "{tail}");
}

#[test]
fn the_schema_records_the_length_field_and_a_slice_type() {
    let checked = check(&with("main(): return\n")).expect("checks");
    let view = checked
        .aggregates
        .iter()
        .find(|aggregate| aggregate.name == "View")
        .unwrap();
    assert_eq!(view.fields[0].length, None);
    assert_eq!(view.fields[1].length, Some(0));
    let SourceType::Array { id } = view.fields[1].ty else {
        panic!("the data field is an array");
    };
    assert_eq!(
        checked.instances[id as usize].head,
        CheckedSourceInstanceHead::Slice
    );
}

#[test]
fn an_index_is_accepted_when_a_guard_proves_it_below_the_length() {
    accepts(
        "get(v ref View, i in 0 to 15) Byte:\n  when i >= v.len then\n    return 0\n  return v.data.[i]\n",
    );
}

#[test]
fn an_index_is_accepted_when_the_domain_proves_it_below_the_length() {
    // `i` is at most 3 and `len` is at least 4.
    accepts(
        "get(v ref View, i in 0 to 3) Byte:\n  when v.len < 4 then\n    return 0\n  return v.data.[i]\n",
    );
}

#[test]
fn an_index_not_proved_below_the_length_is_unproved() {
    rejects(
        "get(v ref View, i in 0 to 15) Byte:\n  return v.data.[i]\n",
        SourceCheckError::Unproved,
    );
    // A guard on the wrong side proves nothing.
    rejects(
        "get(v ref View, i in 0 to 15) Byte:\n  when i <= v.len then\n    return 0\n  return v.data.[i]\n",
        SourceCheckError::Unproved,
    );
    rejects(
        "first(v ref View) Byte:\n  return v.data.[0]\n",
        SourceCheckError::Unproved,
    );
}

#[test]
fn a_guard_on_a_different_length_proves_nothing() {
    rejects(
        "get(v ref View, w ref View, i in 0 to 15) Byte:\n  when i >= w.len then\n    return 0\n  return v.data.[i]\n",
        SourceCheckError::Unproved,
    );
}

#[test]
fn a_view_is_built_from_a_slice_proved_to_fit() {
    accepts(
        "make(storage ref [Byte; 16], n in 0 to 16) View:\n  return View(len: n, data: ref storage.[0; n])\n",
    );
    accepts(
        "make(storage mut [Byte; 16], n in 0 to 16) MutView:\n  return MutView(len: n, data: mut storage.[0; n])\n",
    );
}

#[test]
fn a_slice_longer_than_its_source_is_unproved() {
    rejects(
        "make(storage ref [Byte; 8], n in 0 to 16) View:\n  return View(len: n, data: ref storage.[0; n])\n",
        SourceCheckError::Unproved,
    );
    // A guard that brings the length within the array is enough.
    accepts(
        "make(storage ref [Byte; 8], n in 0 to 16) View:\n  when n > 8 then\n    return View(len: 0, data: ref storage.[0; 0])\n  return View(len: n, data: ref storage.[0; n])\n",
    );
}

#[test]
fn the_slice_length_must_equal_the_length_field() {
    rejects(
        "make(storage ref [Byte; 16], n in 0 to 16, m in 0 to 16) View:\n  return View(len: m, data: ref storage.[0; n])\n",
        SourceCheckError::Type,
    );
    rejects(
        "make(storage ref [Byte; 16], n in 0 to 16) View:\n  return View(len: 3, data: ref storage.[0; n])\n",
        SourceCheckError::Type,
    );
    accepts(
        "make(storage ref [Byte; 16]) View:\n  return View(len: 3, data: ref storage.[0; 3])\n",
    );
    // Equal by a fact the guard teaches.
    accepts(
        "make(storage ref [Byte; 16], n in 0 to 16, m in 0 to 16) View:\n  when m = n then\n    return View(len: m, data: ref storage.[0; n])\n  return View(len: 0, data: ref storage.[0; 0])\n",
    );
}

#[test]
fn the_length_field_is_fixed_once_the_view_is_built() {
    rejects(
        "grow(storage ref [Byte; 16]) View:\n  v: View(len: 3, data: ref storage.[0; 3])\n  v.len :: 4\n  return v\n",
        SourceCheckError::Immutable,
    );
}

#[test]
fn the_length_of_an_array_must_be_a_number() {
    let source = "Flag has up in 0 to 1\nBad has\n  flag Flag\n  data ref [Flag; flag]\n";
    assert_eq!(check(source), Err(SourceCheckError::Type));
    let missing = "Bad has\n  data ref [Flag; nothing]\n";
    assert!(check(missing).is_err());
}

#[test]
fn a_linear_element_cannot_be_copied_out_of_a_borrowed_array() {
    let source = "\
Own has x in 0 to 9
Holder has
  len in 0 to 4
  items ref [Own; len]
take(h ref Holder, i in 0 to 3) Own:
  when i >= h.len then
    return Own(x: 0)
  return h.items.[i]
";
    assert_eq!(check(source), Err(SourceCheckError::MoveOutOfBorrow));
}

#[test]
fn reading_the_same_element_twice_is_fine() {
    accepts(
        "twice(v ref View, i in 0 to 15) Byte:\n  when i >= v.len then\n    return 0\n  a: v.data.[i]\n  return v.data.[i]\n",
    );
}

#[test]
fn a_slice_borrows_its_source() {
    rejects(
        "both(storage mut [Byte; 16], n in 0 to 16):\n  v: View(len: n, data: ref storage.[0; n])\n  w: View(len: n, data: mut storage.[0; n])\n  return\n",
        SourceCheckError::BorrowConflict,
    );
}

#[test]
fn a_view_of_a_local_array_cannot_be_returned() {
    rejects(
        "bad() View:\n  storage [Byte; 2]: [1, 2]\n  return View(len: 2, data: ref storage.[0; 2])\n",
        SourceCheckError::BorrowEscape,
    );
}
