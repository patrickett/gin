//! `ref data.[index]` (a borrow of one element, bounds proved) and raw addresses
//! (`place as @T`, `@place`), which are unchecked from then on.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

fn view(methods: &str) -> String {
    format!(
        "#unrestricted\nLength is in 0 to 4096\nView(element) has\n  len Length\n  data ref [element; len]\n\n{methods}main(): return\n"
    )
}

fn accepts(methods: &str) {
    let source = view(methods);
    assert!(check(&source).is_ok(), "{methods}: {:?}", check(&source).map(|_| ()));
}

fn rejects(methods: &str, error: SourceCheckError) {
    assert_eq!(check(&view(methods)).map(|_| ()), Err(error), "{methods}");
}

#[test]
fn an_element_is_borrowed_when_the_index_is_proved_in_range() {
    accepts(
        "  borrow(self ref, index Length and index < self.len) ref element:\n    return ref self.data.[index]\n",
    );
}

#[test]
fn an_element_borrow_without_a_bounds_proof_is_unproved() {
    rejects(
        "  borrow(self ref, index Length) ref element:\n    return ref self.data.[index]\n",
        SourceCheckError::Unproved,
    );
    rejects(
        "  borrow(self ref, index Length and index <= self.len) ref element:\n    return ref self.data.[index]\n",
        SourceCheckError::Unproved,
    );
}

#[test]
fn an_element_of_an_owned_parameter_cannot_be_returned_borrowed() {
    rejects(
        "  take(self, index Length and index < self.len) ref element:\n    return ref self.data.[index]\n",
        SourceCheckError::BorrowEscape,
    );
}

#[test]
fn the_address_of_a_borrowed_array_is_a_pointer_to_its_elements() {
    accepts("  pointer(self ref) @element: return #unchecked(self.data as @element)\n");
    accepts("  pointer(self ref) @element: return #unchecked((@self.data) as @element)\n");
    accepts("  address(self ref) @Self: return #unchecked(@self)\n");
}

#[test]
fn an_address_has_the_type_of_its_place_or_its_element() {
    rejects(
        "  pointer(self ref) @Length: return #unchecked(self.data as @Length)\n",
        SourceCheckError::Type,
    );
    rejects(
        "  pointer(self ref) @element: return #unchecked(@self.data)\n",
        SourceCheckError::Type,
    );
}

#[test]
fn an_address_records_no_loan() {
    // The pointer is unchecked: the source may be changed while it exists.
    let source = "\
Cell has x in 0 to 9
poke(c mut Cell): return
go():
  c: Cell(x: 1)
  p: #unchecked(@c)
  poke(c)
  q: p
  c2: c
  Cell(x) := c2
  return
";
    let result = check(source).map(|_| ());
    assert_eq!(result, Ok(()));
}

// `#unchecked(...)`: wherever the checker stops proving, the writer says so at that operation.

fn accepted_error(methods: &str) -> Option<SourceCheckError> {
    check(&view(methods)).err()
}

#[test]
fn an_unchecked_address_without_the_marker_is_missing_acceptance() {
    for methods in [
        "  pointer(self ref) @element: return self.data as @element\n",
        "  pointer(self ref) @element: return (@self.data) as @element\n",
        "  address(self ref) @Self: return @self\n",
        "  number(n in 0 to 100) @Length: return n as @Length\n",
        "  number(p @Length) Length: return p as Length\n",
        "  retype(p @Length) @Self: return p as @Self\n",
    ] {
        assert_eq!(accepted_error(methods), Some(SourceCheckError::MissingAcceptance), "{methods}");
    }
}

#[test]
fn the_marker_accepts_each_unchecked_operation() {
    accepts("  number(n in 0 to 100) @Length: return #unchecked(n as @Length)\n");
    accepts("  number(p @Length) Length: return #unchecked(p as Length)\n");
    accepts("  retype(p @Length) @Self: return #unchecked(p as @Self)\n");
    accepts("  a(self ref) @Self:\n    p: #unchecked(@self)\n    return p\n");
}

#[test]
fn the_marker_covers_the_expression_it_follows_and_no_more() {
    // One marker after the whole expression accepts the address and the cast.
    accepts("  both(self ref) Length: return #unchecked((@self) as Length)\n");
    accepts("  both(self ref) Length: return #unchecked(((#unchecked(@self)) as Length))\n");
    // An address outside the marked expression is still unaccepted.
    rejects(
        "  both(self ref) Length:\n    p: @self\n    return #unchecked(p as Length)\n",
        SourceCheckError::MissingAcceptance,
    );
    // No marker at all.
    rejects(
        "  both(self ref) Length: return (@self) as Length\n",
        SourceCheckError::MissingAcceptance,
    );
}

#[test]
fn accepting_does_not_mark_the_function_or_force_its_callers() {
    accepts(
        "  raw(self ref) @Self: return #unchecked(@self)\n  caller(self ref) Length:\n    p: self.raw()\n    return 0\n",
    );
}

#[test]
fn the_marker_on_a_checked_operation_is_an_error() {
    for methods in [
        "  f(n in 0 to 100) Length: return #unchecked(n as Length)\n",
        "  f(n in 0 to 100) Length: return #unchecked(n)\n",
        "  f(n in 0 to 100) Length: return #unchecked((n + 1))\n",
    ] {
        assert_eq!(accepted_error(methods), Some(SourceCheckError::UnneededAcceptance), "{methods}");
    }
}

#[test]
fn the_innermost_marker_accepts_and_an_outer_one_around_it_is_unneeded() {
    // One marker accepts both the address and the cast inside it.
    accepts("  pointer(self ref) @element: return #unchecked((@self.data) as @element)\n");
    // A marker around a marker: the inner one is used, the outer one accepts nothing.
    assert_eq!(
        accepted_error("  pointer(self ref) @element: return #unchecked(#unchecked((@self.data) as @element))\n"),
        Some(SourceCheckError::UnneededAcceptance),
    );
    // An inner marker only around the address leaves the cast unaccepted by it, but the outer
    // marker accepts the cast: both are used.
    accepts("  pointer(self ref) @element: return #unchecked((#unchecked(@self.data)) as @element)\n");
}

#[test]
fn the_directive_takes_exactly_one_positional_expression() {
    for methods in [
        "  f(n in 0 to 100) Length: return #unchecked()\n",
        "  f(n in 0 to 100) Length: return #unchecked(n, n)\n",
        "  f(n in 0 to 100) Length: return #unchecked(value: n)\n",
    ] {
        assert_eq!(accepted_error(methods), Some(SourceCheckError::Type), "{methods}");
    }
}

#[test]
fn checked_casts_stay_unmarked() {
    accepts("  f(n in 0 to 100) Length: return n as Length\n");
}

#[test]
fn a_pointer_cast_to_its_own_type_is_not_unchecked() {
    accepts("  same(p @Length) @Length: return p as @Length\n");
    rejects(
        "  same(p @Length) @Length: return #unchecked(p as @Length)\n",
        SourceCheckError::UnneededAcceptance,
    );
}
