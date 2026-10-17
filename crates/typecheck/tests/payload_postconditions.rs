mod support;
use parser::query::SourceParseExt;
use support::{check, flaw_codes};

const MODEL: &str = "Count is in 0 to 255\nCount has release(self): return\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    release(self): return\nError is Empty or Full\nOutcome is Region or Error\nError has release(self): return\nOutcome has\n    release(self):\n        when self is\n            Region(storage) then storage.release\n            Error(error) then error.release\n        return\n";

#[test]
fn a_success_payload_establishes_empty_storage_and_capacity() {
    let codes = support::flaw_codes_with(
        MODEL,
        "reserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error:\n    return Region(c, 0)\nrun:\n    attempt: reserve(4)\n    when attempt is\n        Region(storage) then\n            require(storage, 4)\n        Error(error) then error.release\n    return\nrequire(storage Region and storage.initialized = 0 and storage.capacity >= c, c Count):\n    storage.release\n    return\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn generic_calls_instantiate_success_payload_types() {
    let source = r#"Byte is in 0 to 255
Byte has release(self): return
#phantom(unit)
Count(unit) is in 0 to 255
Count(unit) has release(self): return
Region(unit) has
    capacity Count(unit)
    initialized Count(unit) and <= capacity
    release(self): return
Error is Empty or Full
Error has release(self): return
reserve(unit, c Count(unit)) storage Region(unit) thus storage.initialized = 0 and storage.capacity >= c or Error:
    return Region(capacity: c, initialized: 0 as Count(unit))
consume(storage Region(Byte) and storage.initialized = 0 and storage.capacity >= 4):
    storage.release
    return
run:
    attempt: reserve(Byte, 4 as Count(Byte))
    when attempt is
        Region(storage) then consume(storage)
        Error(error) then error.release
    return
"#;
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = check(source);
    assert!(flaw_codes(&typed).is_empty(), "{:?}", typed.all_flaws());
}

const STORE: &str = "Store has\n    reserve(self mut, c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error\n    release(self)\n";

#[test]
fn abstract_methods_publish_success_guarantees() {
    let codes = support::flaw_codes_with(
        MODEL,
        &format!(
            "{STORE}run(a mut has Store):\n    attempt: a.reserve(4)\n    when attempt is\n        Region(storage) then consume(storage)\n        Error(error) then error.release\n    return\nconsume(storage Region and storage.initialized = 0 and storage.capacity >= 4):\n    storage.release\n    return\n"
        ),
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn implementations_inherit_contracts_with_renamed_parameters_and_alias_returns() {
    let codes = support::flaw_codes_with(
        MODEL,
        &format!(
            "{STORE}Concrete has Store\n    reserve(self mut, requested Count) Outcome: return Region(requested, 0)\n    release(self): return\n"
        ),
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn implementations_cannot_omit_an_inherited_success_guarantee() {
    let codes = support::flaw_codes_with(
        MODEL,
        &format!(
            "{STORE}Concrete has Store\n    reserve(self mut, requested Count) Outcome:\n        requested.release\n        return Region(4 as Count, 1 as Count)\n    release(self): return\n"
        ),
    );
    assert!(
        codes.iter().any(|code| code == "type-postcondition-failed"),
        "{codes:?}"
    );
}

#[test]
fn forwarding_preserves_success_guarantees() {
    let codes = support::flaw_codes_with(
        MODEL,
        &format!(
            "{STORE}Backing has Store\n    reserve(self mut, requested Count) Outcome: return Region(requested, 0)\n    release(self): return\nConcrete has Store\n    inner Backing\n    reserve(self mut, requested Count) Outcome: return self.inner.reserve(requested)\n    release(self):\n        Concrete(inner) := self\n        inner.release\n        return\n"
        ),
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn conditional_success_returns_are_verified_before_joining_branches() {
    let codes = support::flaw_codes_with(
        MODEL,
        "reserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error:\n    requested is in Count: c\n    c.release\n    return when requested > 0 then Region(requested as Count, 0) else Empty\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn union_moves_preserve_payload_facts() {
    let codes = support::flaw_codes_with(
        MODEL,
        "reserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error: return Region(c, 0)\nrun:\n    original: reserve(4)\n    moved: original\n    when moved is\n        Region(storage) then consume(storage)\n        Error(error) then error.release\n    return\nconsume(storage Region and storage.initialized = 0 and storage.capacity >= 4):\n    storage.release\n    return\n",
    );
    assert!(codes.is_empty(), "{codes:?}");
}

const CASES: &[(&str, &str, &str)] = &[
    (
        "an_invalid_success_payload_is_rejected",
        "reserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error:\n    return Region(0, 0)\n",
        "type-postcondition-unproven",
    ),
    (
        "conditional_success_returns_cannot_hide_invalid_metadata",
        "reserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error:\n    requested is in Count: c\n    c.release\n    return when requested > 0 then Region(4 as Count, 1 as Count) else Empty\n",
        "type-postcondition-failed",
    ),
    (
        "mutation_invalidates_a_success_payload_guarantee",
        "replace(previous Count, next Count) Count: return next\nreserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error: return Region(c, 0)\nrun:\n    attempt: reserve(4)\n    when attempt is\n        Region(storage) then\n            storage.initialized :: replace(storage.initialized, 1)\n            consume(storage)\n        Error(error) then error.release\n    return\nconsume(storage Region and storage.initialized = 0):\n    storage.release\n    return\n",
        "type-parameter-refinement-failed",
    ),
    (
        "success_contracts_refer_to_incoming_owned_arguments",
        "replace(previous Count, next Count) Count: return next\nreserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error:\n    c :: replace(c, 0)\n    return Region(c, 0)\n",
        "type-postcondition-unproven",
    ),
    (
        "abstract_payload_contract_operands_are_typechecked",
        "Store has\n    reserve(self mut, c Count) storage Region thus storage.initialized = missing or Error\n",
        "type-constraint-unknown-name",
    ),
    (
        "another_payload_variant_does_not_receive_success_facts",
        "Other has\n    initialized Count\n    release(self): return\nreserve(c Count) storage Region thus storage.initialized = 0 or Other:\n    c.release\n    return Other(1)\nrun:\n    attempt: reserve(4)\n    when attempt is\n        Region(storage) then storage.release\n        Other(storage) then consume(storage)\n    return\nconsume(storage Other and storage.initialized = 0):\n    storage.release\n    return\n",
        "type-parameter-refinement-unproven",
    ),
    (
        "mutating_a_union_invalidates_its_payload_guarantees",
        "replace_attempt(previous Outcome, next Outcome) Outcome:\n    previous.release\n    return next\nchange(attempt mut Outcome):\n    attempt :: replace_attempt(attempt, Region(4 as Count, 1 as Count))\n    return\nreserve(c Count) storage Region thus storage.initialized = 0 and storage.capacity >= c or Error: return Region(c, 0)\nrun:\n    attempt Outcome: reserve(4)\n    change(attempt)\n    when attempt is\n        Region(storage) then consume(storage)\n        Error(error) then error.release\n    return\nconsume(storage Region and storage.initialized = 0):\n    storage.release\n    return\n",
        "type-parameter-refinement-unproven",
    ),
];

#[test]
fn payload_postcondition_cases() {
    for &(name, body, expected) in CASES {
        let codes = support::flaw_codes_with(MODEL, body);
        assert!(
            codes.iter().any(|code| code == expected),
            "case `{name}` failed: {codes:?}"
        );
    }
}
