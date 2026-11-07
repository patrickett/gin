//! Element access through a borrowed array (`data ref [element; len]`): the bound is proved from
//! the sibling length, a borrow reads without moving, and a linear element is never moved out
//! by index.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict contract fixture");
    check_file(&file, Limits::default())
}

const TOKEN: &str = "Token has\n    n in 0 to 9\n    end(self):\n        Token(n) := self\n        return\n";

fn window(method: &str) -> String {
    format!("{TOKEN}Window(element) has\n    len in 0 to 100\n    data ref [element; len]\n    {method}\n")
}

#[test]
fn a_borrow_of_an_element_below_the_length_is_accepted() {
    let source = window("peek(self ref, index in 0 to 100 and index < self.len) ref element:\n        return ref self.data.[index]");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn an_index_not_below_the_length_is_unproved() {
    let source = window("peek(self ref, index in 0 to 100) ref element:\n        return ref self.data.[index]");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
    let at_length = window("peek(self ref, index in 0 to 100 and index <= self.len) ref element:\n        return ref self.data.[index]");
    assert_eq!(check(&at_length), Err(SourceCheckError::Unproved));
}

#[test]
fn a_linear_element_is_not_moved_out_by_index() {
    let moved = window("steal(self ref and self.len > 0) element:\n        return self.data.[0]");
    assert_eq!(check(&moved), Err(SourceCheckError::MoveOutOfBorrow));
    let copied = window("copy_out(self ref and self.len > 0) element:\n        return deref self.data.[0]");
    assert_eq!(check(&copied), Err(SourceCheckError::MoveOutOfBorrow));
}

#[test]
fn a_write_through_a_ref_view_is_refused() {
    let source = window("poke(self ref and self.len > 0, value element):\n        self.data.[0] :: value\n        return");
    assert!(check(&source).is_err(), "{:?}", check(&source));
}
