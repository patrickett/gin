mod support;

#[test]
fn inherited_integer_copy_generates_a_callable_body() {
    let source = r#"
Copy has
    copy(self ref) Self:
        bits is in Self: self
        return bits as Self
Word is in -128 to 127
Word has Copy
copy_word(value ref Word) Word: return Word.copy(value)
"#;
    let (verified, diagnostics) = support::codegen_verifies(source, "trait_copy.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(verified);
}

#[test]
fn generic_fallible_operator_generates_a_callable_body() {
    let source = r#"
Div(result) has
    #infix('/')
    divide(self, rhs Self) result
Word is in -128 to 127
Outcome is Word or Flaw(Zero or Overflow)
Word has Div(Outcome)
    divide(self is in Self, rhs is in Self): return when rhs = 0 then Flaw(Zero) else (self / rhs) as Self
quotient(left Word, right Word) Outcome: return left / right
"#;
    let (verified, diagnostics) = support::codegen_verifies(source, "trait_divide.gin");
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(verified);
}
