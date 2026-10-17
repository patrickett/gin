mod support;
use typecheck::TypedExprKind;

#[test]
fn bare_member_uses_the_field_type_instead_of_the_enclosing_type() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Buffer has
    finish(self):
        return
    value Word
    read(self ref) Word: return self.value
Envelope has
    finish(self):
        self.bytes.finish
        return
    bytes Buffer
    read(self ref) Word: return self.bytes.read
value: Envelope(bytes: Buffer(value: 7))
result: value.read
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.exprs.kind.iter().any(|kind| matches!(kind, TypedExprKind::FnCall { target, args: Some(args), .. } if target.0.as_str() == "Buffer.read" && args.len() == 1)));
}

#[test]
fn fields_still_project_and_unknown_members_still_fail() {
    let typed = support::check(
        "Word is in 0 to 255\nWord.finish(self):\n    return\nItem has value Word\nItem.finish(self):\n    return\nitem: Item(value: 7)\nresult: item.value\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let typed = support::check(
        "Word is in 0 to 255\nWord.finish(self):\n    return\nItem has value Word\nItem.finish(self):\n    return\nitem: Item(value: 7)\nresult: item.missing\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-unknown-field")
    );
}

#[test]
fn consuming_a_field_still_requires_consuming_the_owned_parent() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Word has
    discard(value Self):
        return
Buffer(element) has
    finish(self):
        return
    value element
    release(self own):
        Buffer(value: value) := self
        element.discard(value)
        return
Envelope has
    finish(self):
        self.bytes.finish
        return
    bytes Buffer(Word)
    release(self own):
        self.bytes.release
        return
"#,
    );
    let flaws = typed.all_flaws();
    assert!(
        flaws
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-owned-param-not-consumed"),
        "{flaws:?}"
    );
    assert!(
        flaws.iter().all(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-owned-param-not-consumed" | "type-owned-value-not-consumed"
        )),
        "{flaws:?}"
    );
}

#[test]
fn generic_member_specialization_keeps_the_concrete_receiver() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Buffer(element) has
    finish(self):
        return
    value element
    read(self ref) element: return self.value
Envelope has
    finish(self):
        self.bytes.finish
        return
    bytes Buffer(Word)
    read(self ref) Word: return self.bytes.read
value: Envelope(bytes: Buffer(value: 7 as Word))
result: value.read
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed
            .defs
            .keys()
            .any(|id| id.0.as_str().starts_with("Buffer.read$specialized$"))
    );
}

#[test]
fn a_method_with_additional_parameters_requires_arguments() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Buffer has
    finish(self):
        return
    value Word
    read(self ref, index Word) Word: return index
value: Buffer(value: 7)
result: value.read
"#,
    );
    assert!(!typed.all_flaws().is_empty());
    assert!(
        !typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-unknown-field")
    );
}

#[test]
fn chained_methods_lower_each_receiver_once() {
    for depth in [1, 4, 12] {
        let typed = support::check(&format!(
            "Word is in 0 to 255\nWord.finish(self):\n    return\nItem has\n    finish(self):\n        return\n    value Word\n    next(self own) Self: return self\nresult(value Item) Item: return value{}\n",
            ".next".repeat(depth)
        ));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let calls = typed.exprs.kind.iter().filter(|kind| matches!(kind, TypedExprKind::FnCall { target, args: Some(_), .. } if target.0.as_str() == "Item.next")).count();
        assert_eq!(calls, depth);
    }
}

#[test]
fn chained_generic_members_can_have_different_instantiations() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Box(element) has
    finish(self):
        return
    value element
    unwrap(self own) element:
        Box(value: value) := self
        return value

run(value Box(Box(Word))) Word: return value.unwrap.unwrap
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(
        typed
            .defs
            .keys()
            .filter(|id| id.0.as_str().starts_with("Box.unwrap$specialized$"))
            .count(),
        2
    );
}

#[test]
fn generic_member_borrows_an_existing_reference_only_once() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Box(element) has
    finish(self):
        return
    value element
    read(self ref) element: return self.value
Envelope has
    finish(self):
        return
    bytes ref Box(Word)
    read(self ref) Word: return self.bytes.read
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn generic_member_on_a_call_does_not_rewrite_the_receiver_call() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Box(element) has
    finish(self):
        return
    value element
    read(self ref) element: return self.value
make Box(Word): Box(value: 7 as Word)
result: make.read
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn mutable_members_borrow_the_nested_receiver() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Word has
    replace(previous Self, next Self) Self: return next
Buffer has
    finish(self):
        return
    value Word
    clear(self mut):
        self.value:: Word.replace(self.value, 0)
        return
Envelope has
    finish(self):
        self.bytes.finish
        return
    bytes Buffer
    clear(self mut):
        self.bytes.clear
        return
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn parenthesized_members_resolve_and_borrow_the_actual_receiver() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Word has
    replace(previous Self, next Self) Self: return next
Buffer(element) has
    finish(self):
        return
    value element
    read(self ref, fallback element) element: return self.value
    write(self mut, value element):
        self.value:: element.replace(self.value, value)
        return
Envelope has
    finish(self):
        self.bytes.finish
        return
    bytes Buffer(Word)
    read(self ref, fallback Word) Word: return self.bytes.read(fallback)
    write(self mut, value Word):
        self.bytes.write(value)
        return
read(value Word) Word: return value
write(value Word):
    return
run:
    buffer Buffer(Word): Buffer(value: 7 as Word)
    buffer.write(9 as Word)
    envelope: Envelope(bytes: buffer)
    envelope.write(11 as Word)
    result: envelope.read(0 as Word)
    envelope.finish
    return
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for member in ["Buffer.read$specialized$", "Buffer.write$specialized$"] {
        assert!(
            typed
                .defs
                .keys()
                .any(|id| id.0.as_str().starts_with(member))
        );
    }
}

#[test]
fn parenthesized_method_receivers_are_evaluated_once() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Box has
    finish(self):
        return
    value Word
    read(self ref, fallback Word) Word: return self.value
make Box: Box(value: 7)
result: make.read(0)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let calls = typed.exprs.kind.iter().filter(|kind| matches!(kind, TypedExprKind::FnCall { target, .. } if target.0.as_str() == "make")).count();
    assert_eq!(calls, 1);
}

#[test]
fn destructured_receivers_and_named_arguments_resolve() {
    let typed = support::check(
        r#"Word is in 0 to 255
Word.finish(self):
    return
Buffer(element) has
    finish(self):
        return
    value element
    read(self ref, fallback element) element: return self.value
Envelope has bytes Buffer(Word)
Envelope.finish(self):
    self.bytes.finish
    return
run(value Envelope) Word:
    Envelope(bytes) := value
    return bytes.read(fallback: 0 as Word)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn overloaded_members_borrow_the_receiver_before_matching_arguments() {
    let typed = support::check(
        r#"Byte is in 0 to 255
Byte.finish(self):
    return
Negative is in -128 to -1
Negative.finish(self):
    return
Box has
    finish(self):
        return
    value Byte
    choose(self ref, value Byte) Byte: return value
    choose(self ref, value Negative) Negative: return value
box: Box(value: 7)
pos: box.choose(12)
neg: box.choose(-2)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
