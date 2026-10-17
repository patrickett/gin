mod support;

use support::{check, flaw_codes};

#[test]
fn iterator_requires_next() {
    let source = format!(
        "{}\n{}Counter has Iterator(Byte)\n    remaining Byte\n    finish(self):\n        return\n",
        test_fixtures::preludes::BYTE_PRELUDE,
        include_str!("../../../modules/core/iter/iter.gin"),
    );
    let typed = check(&source);
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-missing-trait-method"),
        "{:?}",
        typed.all_flaws(),
    );
}

#[test]
fn iterator_implementation_without_fields_needs_only_next() {
    let typed = check(&format!(
        "{}\n{}Empty has Iterator(Byte)\n    next(self mut) Step(Byte): return Done\n",
        test_fixtures::preludes::BYTE_PRELUDE,
        include_str!("../../../modules/core/iter/iter.gin"),
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
