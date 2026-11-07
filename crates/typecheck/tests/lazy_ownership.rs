//! Untouched slot subtrees must keep their origins without enumerating their tags.
use std::fmt::Write;

use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn source(untouched: usize) -> String {
    assert!(untouched <= 6);
    let mut text = String::from(
        "Slot(element) is Empty or Full(element)\n\
         Buffer(element) has\n",
    );
    text.push_str("    target Slot(element)\n");
    for index in 0..untouched {
        writeln!(text, "    rest{index} Slot(element)").unwrap();
    }
    text.push_str("rebuild(element Type, slot Slot(element)) Slot(element):\n");
    text.push_str("    return when slot is\n");
    text.push_str("        Empty then Empty\n");
    text.push_str("        Full(value) then Full(value)\n");
    text.push_str("forward(element Type, buffer Buffer(element)) Buffer(element):\n");
    text.push_str("    Buffer(target");
    for index in 0..untouched {
        write!(text, ", rest{index}").unwrap();
    }
    text.push_str(") := buffer\n");
    text.push_str("    return Buffer(target: rebuild(element, target)");
    for index in 0..untouched {
        write!(text, ", rest{index}").unwrap();
    }
    text.push_str(")\n");
    text
}

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
    limits: Limits,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid lazy-owner fixture");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, limits);
    assert_eq!(source, before);
    result
}

#[test]
fn added_untouched_slots_do_not_exhaust_the_default_budget() {
    for untouched in 0..=6 {
        let checked = check("library.slots", &source(untouched), &[], Limits::default())
            .unwrap_or_else(|error| panic!("{untouched} untouched slots: {error:?}"));
        assert_eq!(checked.checked().functions.len(), 2);
        println!(
            "untouched={untouched} minimum_work={}",
            minimum_work(&source(untouched))
        );
    }
}

fn minimum_work(text: &str) -> u32 {
    let mut lower = 0;
    let mut upper = Limits::default().work_max;
    check("library.slots", text, &[], Limits::default()).unwrap();
    for _ in 0..17 {
        if lower == upper {
            break;
        }
        let work_max = lower + (upper - lower) / 2;
        let limits = Limits {
            work_max,
            ..Limits::default()
        };
        match check("library.slots", text, &[], limits) {
            Ok(_) => upper = work_max,
            Err(SourceCheckError::Budget) => lower = work_max + 1,
            Err(other) => panic!("unexpected error measuring work: {other:?}"),
        }
    }
    assert_eq!(lower, upper);
    assert!(upper > 0);
    upper
}

#[test]
fn untouched_sibling_growth_is_not_the_eager_state_product() {
    let three = minimum_work(&source(3));
    let six = minimum_work(&source(6));
    // A bounded regression envelope, not an asymptotic complexity claim.
    assert!(six <= three * 4, "three={three}, six={six}");
}

fn guarded_source(untouched: usize) -> String {
    let mut text = source(untouched);
    let returned = text.find("    return Buffer(").unwrap();
    text.truncate(returned);
    text.push_str("    return Buffer(target: conditional(element, 0, target)");
    for index in 0..untouched {
        write!(text, ", rest{index}: conditional(element, 0, rest{index})").unwrap();
    }
    text.push_str(")\n");
    text.push_str(
        "conditional(element Type, index in 0 to 1, slot Slot(element)) Slot(element):\n",
    );
    text.push_str("    return when index is\n");
    text.push_str("        0 then slot\n");
    text.push_str("        1 then rebuild(element, slot)\n");
    text
}

#[test]
fn impossible_callee_outcomes_do_not_inspect_forwarded_slots() {
    for untouched in 0..=6 {
        // Every call selects opaque forwarding; the inspecting index=1 body is
        // verified in the library, but is impossible at these particular calls.
        let direct = guarded_source(untouched);
        let nested = format!(
            "{}\nIndex has value in 0 to 1\n",
            direct
                .replace(
                    "conditional(element, 0,",
                    "conditional(element, Index(value: 0),"
                )
                .replace("index in 0 to 1, slot", "index Index, slot")
                .replace(
                    "    return when index is\n",
                    "    Index(number) := index\n    return when number is\n",
                )
        );
        for text in [&direct, &nested] {
            check("library.slots", text, &[], Limits::default())
                .unwrap_or_else(|error| panic!("{untouched} guarded siblings: {error:?}"));
        }
    }
}

#[test]
fn impossible_imported_outcomes_do_not_force_sibling_choices() {
    for untouched in 0..=6 {
        let text = guarded_source(untouched);
        let buffer = text.find("Buffer(element) has").unwrap();
        let rebuild = text.find("rebuild(element Type").unwrap();
        let forward = text.find("forward(element Type").unwrap();
        let conditional = text.find("conditional(element Type").unwrap();
        let library = format!(
            "{}{}{}",
            &text[..buffer],
            &text[rebuild..forward],
            &text[conditional..],
        );
        let client = format!(
            "use library.guards Slot, conditional\n{}{}",
            &text[buffer..rebuild],
            &text[forward..conditional],
        );
        let library = check("library.guards", &library, &[], Limits::default()).unwrap();
        check("app.forward", &client, &[&library], Limits::default())
            .unwrap_or_else(|error| panic!("{untouched} imported guarded siblings: {error:?}"));
    }
}

const OBSERVER: &str = "
use library.slots Slot, Buffer, forward
Scalar has number in 0 to 16
filled(element Type, value element) Slot(element): return Full(value)
vacant(element Type) Slot(element): return Empty
observe(selected in 0 to 16, untouched in 0 to 16) in 0 to 16 and = untouched:
    before: Buffer(
        target: filled(Scalar, Scalar(number: selected)),
        rest0: filled(Scalar, Scalar(number: untouched)),
        rest1: vacant(Scalar),
        rest2: vacant(Scalar),
        rest3: vacant(Scalar),
        rest4: vacant(Scalar),
        rest5: vacant(Scalar)
    )
    Buffer(target, rest0, rest1, rest2, rest3, rest4, rest5) := forward(Scalar, before)
    Full(selected_value) := target
    Full(untouched_value) := rest0
    Scalar(selected_number) := selected_value
    Scalar(untouched_number) := untouched_value
    Empty := rest1
    Empty := rest2
    Empty := rest3
    Empty := rest4
    Empty := rest5
    return untouched_number
";

#[test]
fn untouched_payload_origin_survives_a_body_free_imported_call() {
    let library = check("library.slots", &source(6), &[], Limits::default()).unwrap();
    check("app.observe", OBSERVER, &[&library], Limits::default()).unwrap();
}

#[test]
fn ownership_conserving_sibling_swap_is_not_identity_preserving() {
    let original = source(6);
    let swapped = original.replace(
        "target: rebuild(element, target), rest0",
        "target: rebuild(element, rest0), rest0: target",
    );
    assert_ne!(original, swapped);
    let library = check("library.slots", &swapped, &[], Limits::default()).unwrap();
    assert!(matches!(
        check("app.observe", OBSERVER, &[&library], Limits::default()),
        Err(SourceCheckError::Unproved)
    ));
}

#[test]
fn opaque_children_cannot_be_dropped_or_duplicated() {
    let original = source(6);
    let dropped = original.replace(
        "rebuild(element, target), rest0, rest1",
        "rebuild(element, target), rest0: Empty, rest1",
    );
    assert_ne!(original, dropped);
    assert!(matches!(
        check("library.slots", &dropped, &[], Limits::default()),
        Err(SourceCheckError::Unconsumed)
    ));
    let duplicated = original.replace(
        "rebuild(element, target), rest0, rest1",
        "rebuild(element, target), rest0, rest1: rest0",
    );
    assert_ne!(original, duplicated);
    assert!(matches!(
        check("library.slots", &duplicated, &[], Limits::default()),
        Err(SourceCheckError::UseAfterMove)
    ));
}
