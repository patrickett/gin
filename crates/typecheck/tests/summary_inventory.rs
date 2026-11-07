//! Public, source-only characterization of summary inventory capture and replay.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const LIBRARY: &str = "\
Slot(element) is Empty or Full(element)
Box(element) has value element
identity(element Type, value element) element: return value
pack(element Type, value element) Box(element): return Box(value: value)
unpack(element Type, boxed Box(element)) element:
    Box(value) := boxed
    return value
swap(element Type, values [element; 2]) [element; 2]:
    first: values.[0]
    second: values.[1]
    return [second, first]
rebuild(element Type, slot Slot(element)) Slot(element):
    return when slot is
        Empty then Empty
        Full(value) then Full(value)
";

const ORIGIN_CLIENT: &str = "\
use inventory.library identity, pack, unpack, swap
Number has value in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = second:
    values: swap(Number, [Number(value: first), Number(value: second)])
    boxed: pack(Number, values.[0])
    remaining: values.[1]
    Number(selected) := identity(Number, unpack(Number, boxed))
    Number(other) := remaining
    return selected
";

const GUARDS: &str = "\
use inventory.library Slot, rebuild
Index has value in 0 to 1
Packet(element) has first Slot(element), second Slot(element)
conditional(element Type, first Index, second Index, slot Slot(element)) Slot(element):
    Index(left) := first
    Index(right) := second
    return when left is
        0 then
            return when right is
                0 then slot
                1 then rebuild(element, slot)
        1 then
            return when right is
                0 then rebuild(element, slot)
                1 then slot
";

const GUARD_CLIENT: &str = "\
use inventory.guards Index, Packet, conditional
forward(element Type, flag in 0 to 1, packet Packet(element)) Packet(element):
    Packet(first, second) := packet
    return Packet(
        first: conditional(element, Index(value: flag), Index(value: flag), first),
        second: conditional(element, Index(value: flag), Index(value: flag), second)
    )
";

const SPARSE: &str = "\
Marker has
Peek has values [@Marker; 65536], selected @Marker
peek(values [@Marker; 65536]) Peek:
    selected: values.[12345]
    return Peek(values: values, selected: selected)
";

const SPARSE_CLIENT: &str = "\
use inventory.sparse Marker, Peek, peek
forward(input [@Marker; 65536]) Peek:
    Peek(values, selected) := peek(input)
    return Peek(values: values, selected: selected)
";

const FRESH: &str = "\
Marker has
Choice is Empty or Full(Marker)
Fresh has choice Choice
Number has value in 0 to 16
load(pointer @Fresh) Fresh: return #unchecked(pointer as Fresh)
read(pointer @Number) Number: return #unchecked(pointer as Number)
finish(value Marker):
    Marker := value
    return
";

const FRESH_CLIENT: &str = "\
use inventory.fresh Fresh, load, finish
drain(pointer @Fresh):
    Fresh(choice) := load(pointer)
    when choice is
        Empty then ()
        Full(value) then finish(value)
    return
";

const COVERAGE: &str = "\
Big is A or B or C
Flag is Seen
identity(element Type, value element) element: return value
make() (Big,): return (A,)
";

const COVERAGE_CLIENT: &str = "\
use inventory.coverage Big, Flag, identity, make
observe() Flag:
    values: identity((Big,), make())
    value: values.0
    return when value is
        A then Seen
        B then Seen
        C then Seen
";

fn module(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
    limits: Limits,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict summary fixture");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, limits);
    assert_eq!(source, before);
    result
}

fn accepts(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    module(name, text, dependencies, Limits::default())
        .unwrap_or_else(|error| panic!("{error:?}\n{text}"))
}

fn rejects(text: &str, dependency: &CheckedSourceModule, expected: SourceCheckError) {
    assert_eq!(
        module("case.client", text, &[dependency], Limits::default()).unwrap_err(),
        expected,
        "{text}"
    );
}

#[test]
fn generic_wrappers_and_array_reordering_preserve_original_scalar_identity() {
    let library = accepts("inventory.library", LIBRARY, &[]);
    accepts("case.client", ORIGIN_CLIENT, &[&library]);
    rejects(
        &ORIGIN_CLIENT.replace("return selected", "return other"),
        &library,
        SourceCheckError::Unproved,
    );
}

#[test]
fn capture_and_replay_do_not_duplicate_or_forget_original_children() {
    let library = accepts("inventory.library", LIBRARY, &[]);
    rejects(
        &ORIGIN_CLIENT.replace("remaining: values.[1]", "remaining: values.[0]"),
        &library,
        SourceCheckError::UseAfterMove,
    );
    rejects(
        &ORIGIN_CLIENT.replace("    Number(other) := remaining\n", ""),
        &library,
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn jointly_aliased_guards_keep_forwarded_sum_children_opaque() {
    let library = accepts("inventory.library", LIBRARY, &[]);
    let guards = accepts("inventory.guards", GUARDS, &[&library]);
    drop(library);
    accepts("case.client", GUARD_CLIENT, &[&guards]);
}

#[test]
fn sparse_input_subtrees_and_scalar_children_survive_source_free_calls() {
    let library = accepts("inventory.sparse", SPARSE, &[]);
    accepts("case.client", SPARSE_CLIENT, &[&library]);
    let duplicate = "\
use inventory.sparse Marker, Peek, peek
bad(input [@Marker; 65536]) ([@Marker; 65536], [@Marker; 65536]):
    Peek(values, selected) := peek(input)
    return values, values
";
    rejects(duplicate, &library, SourceCheckError::UseAfterMove);
}

#[test]
fn fresh_outputs_keep_unmaterialized_record_and_sum_layouts() {
    let library = accepts("inventory.fresh", FRESH, &[]);
    accepts("case.client", FRESH_CLIENT, &[&library]);
    rejects(
        &FRESH_CLIENT.replace("Full(value) then finish(value)", "Full(value) then ()"),
        &library,
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn fresh_scalar_outputs_do_not_acquire_unrelated_input_facts() {
    let library = accepts("inventory.fresh", FRESH, &[]);
    let valid = "\
use inventory.fresh Number, read
observe(pointer @Number, other in 0 to 16) in 0 to 16:
    Number(value) := read(pointer)
    return value
";
    accepts("case.client", valid, &[&library]);
    rejects(
        &valid.replace(") in 0 to 16:", ") in 0 to 16 and = other:"),
        &library,
        SourceCheckError::Unproved,
    );
}

#[test]
fn declared_coverage_is_preserved_separately_from_actual_output_origins() {
    let library = accepts("inventory.coverage", COVERAGE, &[]);
    accepts("case.client", COVERAGE_CLIENT, &[&library]);
    rejects(
        &COVERAGE_CLIENT.replace("        C then Seen\n", ""),
        &library,
        SourceCheckError::Type,
    );
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

fn minimum(text: &str, dependency: &CheckedSourceModule, flow: bool) -> u32 {
    let mut low = 0;
    let mut high = if flow {
        Limits::default().flow_work_max
    } else {
        Limits::default().work_max
    };
    accepts("case.client", text, &[dependency]);
    for _ in 0..32 {
        if low == high {
            break;
        }
        let midpoint = low + (high - low) / 2;
        match module("case.client", text, &[dependency], budget(flow, midpoint)) {
            Ok(_) => high = midpoint,
            Err(SourceCheckError::Budget) => low = midpoint + 1,
            Err(error) => panic!("unexpected measurement error: {error:?}"),
        }
    }
    assert_eq!(low, high);
    assert!(low > 0);
    low
}

#[test]
fn summary_inventory_costs_and_inspection_data_match_the_baseline() {
    let library = accepts("inventory.library", LIBRARY, &[]);
    let guards = accepts("inventory.guards", GUARDS, &[&library]);
    let sparse = accepts("inventory.sparse", SPARSE, &[]);
    let fresh = accepts("inventory.fresh", FRESH, &[]);
    let coverage = accepts("inventory.coverage", COVERAGE, &[]);
    for (name, text, dependency, expected_cost) in [
        // 4521 -> 4525: every type name looked up first asks the (empty) alias list.
        // re-pinned 2026-11-04 ((4539, 30) -> (4484, 30)): the fixtures declare unions' own variants, no `Empty`/`Full` records.
        ("origins", ORIGIN_CLIENT, &library, (4484, 30)),
        // re-pinned 2026-11-04 ((7755, 58) -> (7765, 58)): the fixtures declare unions' own variants, no `Empty`/`Full` records.
        ("guards", GUARD_CLIENT, &guards, (7765, 58)),
        ("sparse", SPARSE_CLIENT, &sparse, (1643, 14)),
        // re-pinned 2026-11-04 ((2667, 27) -> (2408, 27)): the fixtures declare unions' own variants, no `Empty`/`Full` records.
        ("fresh", FRESH_CLIENT, &fresh, (2408, 27)),
        // re-pinned 2026-11-04 ((2686, 26) -> (2198, 23)): the fixtures declare unions' own variants, no `Empty`/`Full` records.
        ("coverage", COVERAGE_CLIENT, &coverage, (2198, 23)),
    ] {
        let work = minimum(text, dependency, false);
        let flow = minimum(text, dependency, true);
        assert_eq!((work, flow), expected_cost, "{name}");
        let expected = accepts("case.client", text, &[dependency]);
        let repeated = accepts("case.client", text, &[dependency]);
        assert_eq!(expected.checked(), repeated.checked());
        for (is_flow, limit) in [(false, work), (true, flow)] {
            let checked =
                module("case.client", text, &[dependency], budget(is_flow, limit)).unwrap();
            assert_eq!(checked.checked(), expected.checked());
            assert_eq!(
                module(
                    "case.client",
                    text,
                    &[dependency],
                    budget(is_flow, limit - 1)
                )
                .unwrap_err(),
                SourceCheckError::Budget
            );
        }
    }
}
