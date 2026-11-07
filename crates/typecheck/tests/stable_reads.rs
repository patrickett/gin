//! A method that only reads its receiver (`return self.inner.len`, at any depth) gives the same
//! number each time it is read, until the receiver is written or lent `mut`.
use typecheck::{CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict fixture");
    check_file(&file, Limits::default())
}

const TYPES: &str = "\
Buf has
    n in 0 to 100
    len(self ref) in 0 to 100: return self.n
    add(self mut and self.n < 100) where self.n :: self.n + 1:
        self.n :: self.n + 1
        return
Wrap has
    inner Buf
    len(self ref) in 0 to 100: return self.inner.len
    computed(self ref) in 0 to 200: return self.inner.len + 0
    add(self mut and self.len < 100) where self.len :: self.len + 1:
        self.inner.add
        return
same(a in 0 to 100, b in 0 to 100 and a = b): return
";

fn with(body: &str) -> String {
    format!("{TYPES}{body}")
}

#[test]
fn a_chain_reader_read_twice_is_one_number() {
    let source = with("f(w ref Wrap):\n    same(w.len, w.len)\n    return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_read_after_a_write_is_a_different_number() {
    let source = with("f(w mut Wrap and w.len < 100):\n    before: w.len\n    w.add\n    same(before, w.len)\n    return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
    // Control: the number is known through the method's own transition.
    let known = with("f(w mut Wrap and w.len < 100):\n    before: w.len\n    w.add\n    after: w.len\n    return\n");
    assert!(check(&known).is_ok(), "{:?}", check(&known));
}

#[test]
fn two_receivers_do_not_share_a_number() {
    let source = with("f(a ref Wrap, b ref Wrap):\n    same(a.len, b.len)\n    return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
}

#[test]
fn a_method_that_computes_does_not_get_one() {
    let source = with("f(w ref Wrap):\n    same(w.computed, w.computed)\n    return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unproved));
}

#[test]
fn a_read_after_a_mut_lend_in_a_loop_is_not_the_number_from_before_it() {
    // The loop lends `w` `mut`: what was read before it says nothing of later passes.
    let source = with("f(w mut Wrap and w.len = 0):\n    i in 0 to 3: 0\n    while i < 3 do\n        z in 0 to 0: w.len\n        w.add\n        i :: i + 1\n    return\n");
    assert!(check(&source).is_err(), "{:?}", check(&source));
}
