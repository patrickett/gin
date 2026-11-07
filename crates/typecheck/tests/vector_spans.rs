//! The span layer (`modules/core/span`) and the fixed-capacity Vector over it
//! (`modules/core/vector`), checked as the draft library is written, with negative controls.
//! Element reads and borrows wait on the pointer-regions decision (D1 / S-2) and are not here.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const SPAN: &str = include_str!("../../../modules/core/span/span.gin");
const VECTOR: &str = include_str!("../../../modules/core/vector/vector.gin");

fn module(name: &str, text: &str, deps: &[&CheckedSourceModule]) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source");
    // A package module gets forty times a file's work, as `ginc --check-new` gives it.
    let limits = Limits { work_max: Limits::default().work_max * 40, ..Limits::default() };
    check_module(name, &source, deps, limits)
}

const OPS: &str = "Release has\n    release(self)\n";
// The borrowed array views the span layer hands out, as `core.slice` declares them (`part` is what
// `view_from` is built on).
const SLICE: &str = "max_length := 18446744073709551615\n\n#unrestricted\nLength is in 0 to 18446744073709551615\n\nSliceView(element) has\n    len Length\n    data ref [element; len]\n\n    part(self ref, count in 0 to max_length, from in 0 to max_length and from + count <= self.len) piece SliceView(element) where piece.len = count:\n        return SliceView(len: count, data: ref self.data.[from; count])\n\nSliceMutView(element) has\n    len Length\n    data mut [element; len]\n";

fn client(body: &str) -> Result<(), SourceCheckError> {
    let slice = module("core.slice", SLICE, &[]).expect("core.slice checks");
    let span = module("core.span", SPAN, &[&slice]).expect("core.span checks");
    let ops = module("core.ops", OPS, &[]).expect("core.ops checks");
    let vector = module("core.vector", VECTOR, &[&span, &ops, &slice]).expect("core.vector checks");
    let text = format!(
        "use core.span Vacant\nuse core.vector Vector\nuse core.ops Release\n\nToken has Release\n    id in 0 to 9\n    end(self):\n        Token(id) := self\n        return\n    release(self):\n        Token(id) := self\n        return\n\nPlain has\n    id in 0 to 9\n\n{body}\n"
    );
    module("app", &text, &[&span, &ops, &vector, &slice]).map(|_| ())
}

#[test]
fn the_span_layer_and_the_vector_check() {
    let slice = module("core.slice", SLICE, &[]).unwrap();
    let span = module("core.span", SPAN, &[&slice]).unwrap();
    let ops = module("core.ops", OPS, &[]).unwrap();
    module("core.vector", VECTOR, &[&span, &ops, &slice]).unwrap();
}

#[test]
fn a_release_that_skips_its_items_is_refused() {
    // Negative control on the library itself: without the loop, the live values are dropped.
    let slice = module("core.slice", SLICE, &[]).unwrap();
    let span = module("core.span", SPAN, &[&slice]).unwrap();
    let ops = module("core.ops", OPS, &[]).unwrap();
    let skipping = VECTOR.replace(
        "        while rest.length > 0 do\n            value: rest.pop\n            item.release(value)\n",
        "",
    );
    assert_ne!(skipping, VECTOR);
    assert!(module("core.vector", &skipping, &[&span, &ops, &slice]).is_err());
}

#[test]
fn an_item_type_without_release_is_refused_at_the_bound() {
    let plain = "demo(storage Vacant(Plain) and storage.start = 0) Vacant(Plain):\n    v: Vector.new(Plain, storage)\n    return v.release";
    assert_eq!(client(plain), Err(SourceCheckError::Type));
}

#[test]
fn push_and_pop_within_capacity_are_proved() {
    let ok = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 2) Vacant(Token):\n    v: Vector.new(Token, storage)\n    v.push(Token(id: 1))\n    v.push(Token(id: 2))\n    second: v.pop\n    first: v.pop\n    second.end\n    first.end\n    return v.release";
    assert_eq!(client(ok), Ok(()));
}

#[test]
fn push_at_capacity_is_unproved() {
    let full = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 1) Vacant(Token):\n    v: Vector.new(Token, storage)\n    v.push(Token(id: 1))\n    v.push(Token(id: 2))\n    b: v.pop\n    a: v.pop\n    b.end\n    a.end\n    return v.release";
    assert_eq!(client(full), Err(SourceCheckError::Unproved));
}

#[test]
fn pop_of_an_empty_vector_is_unproved() {
    let empty = "demo(storage Vacant(Token) and storage.start = 0) Vacant(Token):\n    v: Vector.new(Token, storage)\n    a: v.pop\n    a.end\n    return v.release";
    assert_eq!(client(empty), Err(SourceCheckError::Unproved));
}

#[test]
fn taking_the_same_value_twice_is_unproved() {
    let twice = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 2) Vacant(Token):\n    v: Vector.new(Token, storage)\n    v.push(Token(id: 1))\n    a: v.pop\n    b: v.pop\n    a.end\n    b.end\n    return v.release";
    assert_eq!(client(twice), Err(SourceCheckError::Unproved));
}

#[test]
fn values_left_in_a_vector_are_never_dropped() {
    // Releasing a vector that still holds a value ends the value through `Release`.
    let ended = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 2) Vacant(Token):\n    v: Vector.new(Token, storage)\n    v.push(Token(id: 1))\n    return v.release";
    assert_eq!(client(ended), Ok(()));
    // Dropping the vector itself is refused: it is linear.
    let dropped = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 2):\n    v: Vector.new(Token, storage)\n    v.push(Token(id: 1))\n    return";
    assert_eq!(client(dropped), Err(SourceCheckError::Unconsumed));
}

#[test]
fn a_released_vector_cannot_be_used() {
    let after = "demo(storage Vacant(Token) and storage.start = 0 and storage.length = 2) Vacant(Token):\n    v: Vector.new(Token, storage)\n    rest: v.release\n    v.push(Token(id: 1))\n    return rest";
    assert!(matches!(client(after), Err(SourceCheckError::UseAfterMove | SourceCheckError::Unconsumed)), "{:?}", client(after));
}

#[test]
fn the_span_layer_hides_its_representation() {
    let slice = module("core.slice", SLICE, &[]).unwrap();
    let span = module("core.span", SPAN, &[&slice]).unwrap();
    // A span cannot be built from outside: its fields are private.
    let forge = "use core.span Vacant, Address\nforge(item Type) Vacant(item): return Vacant(base: 4096, first: 0, count: 10)\n";
    assert_eq!(module("app", forge, &[&span]).map(|_| ()), Err(SourceCheckError::Privacy));
    // Nor can a slot be handed back to a vacant span by anyone but `take_last`.
    let give = "use core.span Vacant\ngrow(item Type, v mut Vacant(item) and v.start > 0):\n    v.give_front(v.start - 1)\n    return\n";
    assert!(module("app", give, &[&span]).is_err());
}
