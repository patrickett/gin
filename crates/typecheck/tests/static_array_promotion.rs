mod support;

use support::{check, flaw_codes};

const PRELUDE: &str = "Byte is in 0 to 255\nByte has finish(self): return\n";

#[test]
fn immutable_literal_arrays_can_outlive_the_call() {
    for source in [
        "literal(seed Byte) ref [Byte; 2]: return ref [10 as Byte, 20 as Byte]",
        "literal(seed Byte) ref [Byte; 2]: return ref [10 as Byte; 2]",
        "literal(seed ref Byte) ref [Byte; 2]: return ref [10 as Byte, 20 as Byte]",
        "literal(seed Byte) ref [[Byte; 2]; 2]: return ref [[10 as Byte, 20 as Byte], [30 as Byte, 40 as Byte]]",
        "View has\n    length Byte\n    data ref [Byte; length]\n    finish(self): return\nliteral(seed Byte) View:\n    data: ref [10 as Byte, 20 as Byte]\n    return View(length: 2 as Byte, data: data)",
    ] {
        let typed = check(&format!("{PRELUDE}{source}\n"));
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn runtime_and_mutable_array_storage_cannot_escape() {
    for source in [
        "literal(seed Byte) ref [Byte; 2]: return ref [seed, 20 as Byte]",
        "literal(seed Byte) mut [Byte; 2]: return mut [10 as Byte, 20 as Byte]",
        "literal(seed Byte) ref [Byte; 2]:\n    local [Byte; 2]: [10, 20]\n    return ref local",
    ] {
        let typed = check(&format!("{PRELUDE}{source}\n"));
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-reference-return-")),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn static_results_do_not_inherit_unused_input_lifetimes() {
    let typed = check(&format!(
        "{PRELUDE}literal(seed ref Byte) ref [Byte; 2]: return ref [10 as Byte, 20 as Byte]\nforward(seed Byte) ref [Byte; 2]: return literal(seed)\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_return_can_choose_static_or_caller_owned_storage() {
    let typed = check(&format!(
        "{PRELUDE}choose(flag Byte, data ref [Byte; 2]) ref [Byte; 2]: return when flag is 0 then ref [10 as Byte, 20 as Byte] else data\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn an_empty_literal_array_has_static_storage() {
    let typed = check(&format!(
        "{PRELUDE}empty(seed Byte) ref [Byte; 0]: return ref []\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
