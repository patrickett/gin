mod support;

use support::check;

const TYPES: &str = r#"Word is in 0 to 255
Other is in 0 to 255
#phantom(unit)
Count(unit) is in 0 to 65535
Allocation(element) has capacity Count(element)
AllocError is OutOfMemory or AddressSpaceLimitExceeded
Word has
    release(self):
        return
Other has
    release(self):
        return
Count(unit) has
    release(self):
        return
Allocation(element) has
    release(self):
        return
"#;

#[test]
fn method_infers_element_from_phantom_count_and_specializes_union_return() {
    let typed = check(&format!(
        "{TYPES}{}",
        r#"Allocator has
    reserve(self mut, c Count(x)) Allocation(x) or AllocError
    release(self)
Store has Allocator
    reserve(self mut, c Count(x)) Allocation(x) or AllocError:
        allocation Allocation(x): Allocation(capacity: c)
        return allocation
    release(self):
        return
run:
    store: Store()
    allocation: store.reserve(4 as Count(Word))
    store.release
    return allocation
"#
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let specialized = typed.defs.values().find(|bind| {
        bind.name.as_str().starts_with("Store.reserve$specialized$")
            && !bind.requires_specialization()
    });
    assert!(specialized.is_some(), "{:#?}", typed.defs);
}

#[test]
fn repeated_implicit_type_parameter_rejects_conflicting_units() {
    let typed = check(&format!(
        "{TYPES}{}",
        r#"same(left Count(x), right Count(x)) Count(x): return left
bad: same(1 as Count(Word), 2 as Count(Other))
"#
    ));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-generic-specialization" }),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn implicit_type_parameters_support_named_arguments_and_body_type_operations() {
    let typed = check(&format!(
        "{TYPES}{}",
        r#"capacity(first Count(x), second Count(x)) Count(x):
    width: #size(x)
    return second
good: capacity(second: 2 as Count(Word), first: 1 as Count(Word))
"#
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(|bind| {
        bind.name.as_str().starts_with("capacity$specialized$") && !bind.requires_specialization()
    }));
}

#[test]
fn implicit_type_parameters_in_nested_records_are_inferred() {
    let typed = check(&format!(
        "{TYPES}{}",
        r#"Envelope(element) has allocation Allocation(element)
Envelope(element) has
    release(self):
        return
unwrap(envelope Envelope(x)) Allocation(x):
    Envelope(allocation) := envelope
    return allocation
good: unwrap(Envelope(allocation: Allocation(capacity: 3 as Count(Word))))
"#
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(typed.defs.values().any(|bind| {
        bind.name.as_str().starts_with("unwrap$specialized$") && !bind.requires_specialization()
    }));
}

#[test]
fn concrete_type_arguments_do_not_become_implicit_parameters() {
    let typed = check(&format!(
        "{TYPES}{}",
        "capacity(c Count(Word)) Count(Word): return c\ngood: capacity(1 as Count(Word))\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        !typed.defs[&typecheck::DefId::named(internment::Intern::from_ref("capacity"))]
            .requires_specialization()
    );
}
