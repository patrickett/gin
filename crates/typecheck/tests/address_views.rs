//! `#unchecked(address as ref [item; n])`: the one trusted step from an origin to an array.
//! The view borrows the place the address is read from, so nothing that changes or ends that place
//! can run while the view lives.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

const PRELUDE: &str = "\
#unrestricted
Byte is in 0 to 255
#unrestricted
Address is in 0 to 18446744073709551615
Win has
    len in 0 to 100
    data ref [Byte; len]
    at(self ref, index in 0 to 100 and index < self.len) Byte:
        return deref self.data.[index]
WinMut has
    len in 0 to 100
    data mut [Byte; len]
    put(self mut, index in 0 to 100 and index < self.len, value Byte):
        self.data.[index] :: value
        return
";

fn holder(methods: &str) -> String {
    format!(
        "{PRELUDE}Holder has\n    #private\n    base Address\n    #private\n    count in 0 to 100\n    length(self ref) in 0 to 100: return self.count\n    grow(self mut and self.count < 100) where self.count :: self.count + 1:\n        self.count :: self.count + 1\n        return\n    end(self):\n        Holder(base, count) := self\n        return\n{methods}"
    )
}

const PEEK: &str = "    peek(self ref) data Win where data.len = self.length:\n        region: #unchecked(self.base as ref [Byte; self.count])\n        return Win(len: self.count, data: region)\n    poke(self mut) data WinMut where data.len = self.length:\n        region: #unchecked(self.base as mut [Byte; self.count])\n        return WinMut(len: self.count, data: region)\n";

fn client(body: &str) -> String {
    format!("{}use_it(h mut Holder and h.length > 0):\n{body}", holder(PEEK))
}

#[test]
fn a_view_of_an_origin_is_accepted_and_indexed_with_the_ordinary_bound() {
    let source = client("    v: h.peek\n    b: v.at(0)\n    return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn the_cast_needs_its_marker() {
    let unmarked = holder(&PEEK.replace("#unchecked(self.base as ref [Byte; self.count])", "self.base as ref [Byte; self.count]").replace("#unchecked(self.base as mut [Byte; self.count])", "self.base as mut [Byte; self.count]"));
    assert_eq!(check(&unmarked), Err(SourceCheckError::MissingAcceptance));
}

#[test]
fn a_number_or_a_linear_origin_is_not_an_address() {
    let plain = holder(&PEEK.replace("base Address", "base in 0 to 1000"));
    let plain = plain.replace("Holder has\n    #private\n    base Address", "Holder has\n    #private\n    base in 0 to 1000");
    assert_eq!(check(&plain), Err(SourceCheckError::Type));
}

#[test]
fn a_ref_view_cannot_become_a_mut_view() {
    // `poke` on a `ref` receiver would need a `mut` borrow of a shared place.
    let shared = holder(&PEEK.replace("poke(self mut)", "poke(self ref)"));
    assert!(check(&shared).is_err(), "{:?}", check(&shared));
}

#[test]
fn the_view_cannot_outlive_a_change_to_its_owner() {
    let grown = client("    v: h.peek\n    h.grow\n    b: v.at(0)\n    return\n");
    assert_eq!(check(&grown), Err(SourceCheckError::BorrowConflict));
    // Control: with the view finished (in a callee) before the change, it is fine.
    let after = format!(
        "{}first(h ref Holder and h.length > 0):\n    v: h.peek\n    b: v.at(0)\n    return\nuse_it(h mut Holder and h.length > 0 and h.length < 100):\n    first(h)\n    h.grow\n    return\n",
        holder(PEEK)
    );
    assert!(check(&after).is_ok(), "{:?}", check(&after));
}

#[test]
fn a_mut_view_cannot_start_while_a_view_lives() {
    let both = client("    v: h.peek\n    w: h.poke\n    b: v.at(0)\n    return\n");
    assert_eq!(check(&both), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn the_owner_cannot_end_while_a_view_lives() {
    let ended = format!(
        "{}use_it(h Holder and h.length > 0):\n    v: h.peek\n    h.end\n    b: v.at(0)\n    return\n",
        holder(PEEK)
    );
    assert!(check(&ended).is_err(), "{:?}", check(&ended));
}

#[test]
fn a_write_through_a_ref_view_is_refused_and_through_a_mut_view_is_not() {
    let through_ref = format!(
        "{}use_it(h mut Holder and h.length > 0):\n    v: h.peek\n    v.data.[0] :: 1\n    return\n",
        holder(PEEK)
    );
    assert!(check(&through_ref).is_err(), "{:?}", check(&through_ref));
    let started = client("    w: h.poke\n    return\n");
    assert!(check(&started).is_ok(), "{:?}", check(&started));
}
