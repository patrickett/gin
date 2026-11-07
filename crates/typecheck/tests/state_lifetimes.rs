//! Characterization for the state-lifetime refactor, through the public source interface.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const SCALARS: &str = "\
first(value in 0 to 16) in 0 to 16 and = value: return value
second(value in 16 to 32) in 16 to 32 and = value: return value
third(value in -16 to 0) in -16 to 0 and = value: return value
";

const WORLDS: &str = "\
Slot(element) is Empty or Full(element)
identity(element Type, value element) element: return value
rebuild(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
swap(element Type, pair (Slot(element), Slot(element))) (Slot(element), Slot(element)):
    first: pair.0
    second: pair.[1]
    return (rebuild(element, second), rebuild(element, first))
";

const INTEGERS: &str = "\
choose(index in 0 to 2, first in 0 to 16, second in 0 to 16) in 0 to 16:
    return when index is
        0 then first
        1 then second
        2 then first
same(value in 0 to 16) in 0 to 16 and = value:
    return choose(0, value, value)
";

const LIBRARY: &str = "\
Box(element) has value element
identity(element Type, value element) element: return value
pack(element Type, value element) Box(element):
    return Box(value: identity(element, value))
unpack(element Type, boxed Box(element)) element:
    Box(value) := boxed
    return identity(element, value)
";

const CLIENT: &str = "\
use dep.library Box, pack, unpack
Scalar has value in 0 to 32
pass(element Type, value element) element:
    return unpack(element, pack(element, value))
observe(first in 0 to 32, second in 0 to 32) in 0 to 32 and = second:
    Scalar(value) := pass(Scalar, Scalar(value: second))
    return value
";

fn compile(
    text: &str,
    dependencies: &[&CheckedSourceModule],
    limits: Limits,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict state-lifetime fixture");
    let before = source.clone();
    let result = check_module("case.fixture", &source, dependencies, limits);
    assert_eq!(source, before);
    result
}

fn dependency() -> CheckedSourceModule {
    let source = parser::parse_strict(LIBRARY).unwrap();
    check_module("dep.library", &source, &[], Limits::default()).unwrap()
}

fn minimum(text: &str, dependencies: &[&CheckedSourceModule], flow: bool) -> u32 {
    let defaults = Limits::default();
    let mut low = 0;
    let mut high = if flow {
        defaults.flow_work_max
    } else {
        defaults.work_max
    };
    compile(text, dependencies, defaults).unwrap();
    for _ in 0..32 {
        if low == high {
            break;
        }
        let midpoint = low + (high - low) / 2;
        let limits = budget(flow, midpoint);
        match compile(text, dependencies, limits) {
            Ok(_) => high = midpoint,
            Err(SourceCheckError::Budget) => low = midpoint + 1,
            Err(error) => panic!("non-budget error during measurement: {error:?}"),
        }
    }
    assert_eq!(low, high);
    assert!(low > 0);
    low
}

fn budget(flow: bool, amount: u32) -> Limits {
    if flow {
        Limits {
            flow_work_max: amount,
            ..Limits::default()
        }
    } else {
        Limits {
            work_max: amount,
            ..Limits::default()
        }
    }
}

#[test]
fn shared_work_and_flow_boundaries_are_characterized_before_state_moves() {
    let dependency = dependency();
    let cases = [
        ("scalars", SCALARS, &[][..], (834, 3)),
        // 7218 -> 7219: every type name looked up first asks the (empty) alias list.
        // 7247 -> 6703 (flow 138 -> 135), 2026-11-04: `Slot` declares its own variants, so the
        // fixture no longer declares the records `Empty` and `Full`.
        ("worlds", WORLDS, &[][..], (6703, 135)),
        // 2226 -> 2227: a constant stored by a transition is given an identity (one charge).
        ("integers", INTEGERS, &[][..], (2227, 12)),
        ("imports", CLIENT, &[&dependency][..], (2607, 19)),
    ];
    for (name, text, dependencies, expected) in cases {
        let work = minimum(text, dependencies, false);
        let flow = minimum(text, dependencies, true);
        assert_eq!((work, flow), expected, "{name}");
        for (is_flow, limit) in [(false, work), (true, flow)] {
            compile(text, dependencies, budget(is_flow, limit)).unwrap();
            assert_eq!(
                compile(text, dependencies, budget(is_flow, limit - 1)).unwrap_err(),
                SourceCheckError::Budget
            );
        }
    }
}

#[test]
fn repeated_checks_publish_identical_inspection_data() {
    let dependency = dependency();
    for (text, dependencies) in [
        (SCALARS, &[][..]),
        (WORLDS, &[][..]),
        (INTEGERS, &[][..]),
        (CLIENT, &[&dependency][..]),
    ] {
        let expected = compile(text, dependencies, Limits::default()).unwrap();
        for _ in 0..3 {
            let actual = compile(text, dependencies, Limits::default()).unwrap();
            assert_eq!(actual.checked(), expected.checked());
        }
    }
}

#[test]
fn earlier_function_facts_do_not_prove_a_later_functions_contract() {
    let text = "\
first(left in 0 to 16, right in 0 to 16 and >= left) in 0 to 16 and >= left:
    return right
second(left in 0 to 16, right in 0 to 16) in 0 to 16 and >= left:
    return right
";
    assert_eq!(
        compile(text, &[], Limits::default()).unwrap_err(),
        SourceCheckError::Unproved
    );
}

#[test]
fn earlier_names_and_generics_are_not_visible_to_later_bodies() {
    for (text, expected) in [
        (
            "Marker has
first(value Marker) Marker: return value
second() Marker: return value
",
            SourceCheckError::UnknownName,
        ),
        (
            "identity(element Type, value element) element: return value
second(value element) element: return value
",
            SourceCheckError::Unsupported,
        ),
    ] {
        assert_eq!(
            compile(text, &[], Limits::default()).unwrap_err(),
            expected,
            "{text}"
        );
    }
}

#[test]
fn imported_calls_keep_original_scalar_origins_after_multiple_bodies() {
    let dependency = dependency();
    compile(CLIENT, &[&dependency], Limits::default()).unwrap();
    let changed = CLIENT.replace("Scalar(value: second)", "Scalar(value: first)");
    assert_eq!(
        compile(&changed, &[&dependency], Limits::default()).unwrap_err(),
        SourceCheckError::Unproved
    );
}
