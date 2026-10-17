mod support;

#[test]
fn generic_record_values_inject_into_type_sums() {
    let typed = support::check(&format!(
        "{}Block(element) has\n    value element\n    finish(self):\n        return\nError is Empty or Full\nAttempt(element) is Block(element) or Failed(Error)\nAttempt(element) has\n    finish(self):\n        return\nmake(element, value element) Attempt(element):\n    block Block(element): Block(value: value)\n    return block\n\nrun: make(Word, 7 as Word)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn cast_generic_arguments_preserve_declared_parameter_names() {
    let typed = support::check(&format!(
        "{}#phantom(unit)\nCount(unit) is in 0 to 65535\nCount(unit) has\n    finish(self):\n        return\nHolder has\n    finish(self):\n        return\n    reserve(self mut, element, c Count(element)) Count(element): return c\n\nrun:\n    holder: Holder()\n    return Holder.reserve(holder, Word, 4 as Count(Word))\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(|bind| {
        bind.name.as_str().contains("Holder.reserve$specialized$")
            && !bind.requires_specialization()
    }));
}

#[test]
fn generic_method_returns_its_element_type() {
    let typed = support::check(&format!(
        "{}    copy(self ref) Self: return deref self\n\nHolder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value element) Self: return Self(value: value)\n    copy_value(self ref) element: return element.copy(self.value)\n\nrun:\n    holder: Holder.new(Word, 7 as Word)\n    value is in Word: Holder.copy_value(holder)\n    return value\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn named_type_arguments_are_removed_without_reordering_runtime_arguments() {
    let typed = support::check(&format!(
        "{}Holder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value element) Self: return Self(value: value)\n\nvalue: Holder.new(value: 7 as Word, element: Word)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn explicit_element_type_must_match_the_value() {
    let typed = support::check(&format!(
        "{}Other is in 0 to 255\nHolder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value element) Self: return Self(value: value)\n\nvalue: Holder.new(Word, 7 as Other)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-generic-specialization"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn element_operations_are_checked_on_instantiation() {
    let typed = support::check(&format!(
        "{}Holder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value element) Self: return Self(value: value)\n    copy_value(self ref) element: return element.copy(self.value)\n\nrun:\n    value: Holder.new(Word, 7 as Word)\n    return Holder.copy_value(value)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(!typed.all_flaws().is_empty());
}

#[test]
fn inferred_integer_arguments_share_the_runtime_domain() {
    let typed = support::check(
        r#"Range(element) has
    finish(self):
        return
    start element
    end element
    new(start element, end element) Range(element): return Self(start: start, end: end)

value: Range.new(12, 1200)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(
        |bind| bind.name.as_str().contains("$specialized$") && !bind.requires_specialization()
    ));
}

#[test]
fn static_generic_methods_only_specialize_parameters_they_use() {
    let typed = support::check(&format!(
        "{}DefaultStore has\n    marker Word\n    finish(self):\n        return\nHolder(element, store: DefaultStore) has\n    finish(self):\n        return\n    value element\n    allocator store\n    echo(element, value element) element: return value\n\nrun: Holder.echo(Word, 7 as Word)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(|bind| {
        bind.name.as_str().contains("Holder.echo$specialized$") && !bind.requires_specialization()
    }));
}

#[test]
fn generic_member_calls_remove_explicit_type_arguments() {
    let typed = support::check(
        r#"Word is in 0 to 255
Holder(element) has
    finish(self):
        return
    value element
    new(element, value element) Self: return Self(value: value)
    convert(self ref, result, value result) Holder(result): return Holder(value: value)
    duplicate(self ref) Self: return self.convert(element, element.copy(self.value))
Word has
    finish(self):
        return
    copy(self ref) Self: return deref self

run:
    holder: Holder.new(Word, 7 as Word)
    return holder.duplicate
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn defaulted_receiver_parameters_remain_generic_in_methods() {
    let typed = support::check(&format!(
        "{}DefaultStore has\n    finish(self):\n        return\n    marker Word\n    copy(self ref) Self: return Self(marker: self.marker)\nCustomStore has\n    finish(self):\n        return\n    marker Word\n    copy(self ref) Self: return Self(marker: self.marker)\nHolder(store: DefaultStore) has\n    finish(self):\n        self.allocator.finish\n        return\n    allocator store\n    new(a own store) Self: return Self(allocator: a)\n    copy(self ref) Self: return Self(allocator: self.allocator.copy)\n\nrun:\n    a: Holder.new(CustomStore(marker: 7))\n    result: a.copy\n    a.finish\n    return result\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn generic_infix_methods_specialize_concrete_receiver_arguments() {
    let typed = support::check(&format!(
        "{}DefaultStore has\n    marker Word\n    finish(self):\n        return\nHolder(store: DefaultStore) has\n    finish(self):\n        self.allocator.finish\n        return\n    allocator store\n    new(a own store) Self: return Self(allocator: a)\n    #infix('+')\n    join(self own, other Self) Self:\n        other.finish\n        return self\n\nrun:\n    left: Holder.new(DefaultStore(marker: 1))\n    right: Holder.new(DefaultStore(marker: 2))\n    return left + right\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn allocator_copy_is_required_only_when_copying_without_a_destination() {
    let declarations = format!(
        "{}DefaultStore has\n    finish(self):\n        return\n    copy(self ref) Self: return Self()\nUniqueStore has\n    finish(self):\n        return\nBuffer(store: DefaultStore) has\n    allocator store\n    new(a own store) Self: return Self(allocator: a)\n    copy(self ref) Self: return Self(allocator: self.allocator.copy)\n    copy_into(self ref, destination, a own destination) Buffer(destination): return Buffer(allocator: a)\n    finish(self):\n        return\n",
        test_fixtures::preludes::WORD_PRELUDE
    );
    let copied_into = support::check(&format!(
        "{declarations}\nrun:\n    value: Buffer.new(UniqueStore())\n    result: value.copy_into(DefaultStore, DefaultStore())\n    value.finish\n    return result\n"
    ));
    assert!(
        copied_into.all_flaws().is_empty(),
        "{:?}",
        copied_into.all_flaws()
    );
    let copied = support::check(&format!(
        "{declarations}\nrun:\n    value: Buffer.new(UniqueStore())\n    return value.copy\n"
    ));
    assert!(
        copied.all_flaws().iter().any(|(_, flaw)| matches!(
            flaw.code.slug().as_str(),
            "type-unknown-field" | "type-unknown-symbol"
        )),
        "{:?}",
        copied.all_flaws()
    );
}

#[test]
fn explicit_type_arguments_resolve_defaulted_records_and_union_members() {
    let typed = support::check(&format!(
        "{}Result is Word or Failed\nResult has\n    finish(self):\n        return\nDefaultStore is ()\nText(store: DefaultStore) has\n    marker Word\n    finish(self):\n        return\nHolder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value own element) Self: return Self(value)\n\nrun:\n    text: Holder.new(Text, Text(marker: 7))\n    word: Holder.new(Word, 7 as Word)\n    return word\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn static_methods_preserve_receiver_parameters_used_in_the_body() {
    let typed = support::check(&format!(
        "{}DefaultStore is ()\nHolder(element, store: DefaultStore) has\n    finish(self):\n        return\n    value element\n    allocator store\n    width() Word: return #size(element) as Word\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let method = typed
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "Holder.width")
        .unwrap();
    assert_eq!(
        method
            .type_parameters()
            .into_iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>(),
        vec!["element"]
    );
}
