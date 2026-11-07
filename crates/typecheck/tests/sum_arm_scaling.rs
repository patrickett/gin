//! A `when` over a sum of variants whose fields are themselves sums.
//!
//! Every arm of the match is checked in some world, because the subject's member is a
//! replay choice and each member gets a world where its arm is the selected one. An arm
//! that is *not* selected in the current world therefore needs a separate hypothetical
//! check only when this `when` did not choose the member itself (the member was already
//! fixed, e.g. by construction). Doing the hypothetical check anyway multiplies the worlds
//! of every arm together: with three sum-typed operands per arm, 27 per arm, 27^n for n.
//!
//! The scaling test states the cost model we want. The soundness tests state what any
//! faster strategy must keep rejecting.
use typecheck::{Limits, SourceCheckError, check_module};

// `Spare` carries a linear `Pin`, so `Reg` is linear (a payload-less union is linear too unless
// marked `#unrestricted`; the payload keeps the fixture honest about `eat`'s arm that consumes it).
const REGISTERS: &str = "\
Pin has n in 0 to 1
Reg is A or B or C or Spare(Pin)

-- consumes one operand; its three outcomes are guarded by which register it is
eat(register Reg) in 0 to 2:
    return when register is
        A then 0
        B then 1
        C then 2
        Spare(pin) then
            Pin(n) := pin
            return 2
";

fn operand_names(arity: usize) -> Vec<String> {
    ["x", "y", "z"][..arity]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

/// A sum of `arms` variants `I{n}` with `arity` register operands each, and `decode`.
fn decoder(arms: usize, arity: usize, bad_arm: Option<usize>) -> String {
    decoder_with(arms, arity, bad_arm, Fault::Duplicate)
}

/// How a bad arm misuses its operands.
#[derive(Clone, Copy)]
enum Fault {
    /// Moves the first operand twice.
    Duplicate,
    /// Never consumes the last operand.
    Drop,
}

fn decoder_with(arms: usize, arity: usize, bad_arm: Option<usize>, fault: Fault) -> String {
    let names = operand_names(arity);
    let mut text = REGISTERS.to_owned();
    let fields: Vec<_> = names.iter().map(|name| format!("{name} Reg")).collect();
    let members: Vec<_> = (0..arms)
        .map(|arm| format!("I{arm}({})", fields.join(", ")))
        .collect();
    text += &format!("Instruction is {}\n\n", members.join(" or "));
    text += &format!("decode(instruction Instruction) in 0 to {}:\n", 2 * arity + 2);
    text += "    return when instruction is\n";
    for arm in 0..arms {
        let binders = names.join(", ");
        let sum: Vec<_> = names.iter().map(|n| format!("eat({n})")).collect();
        let mut body = sum.join(" + ");
        if bad_arm == Some(arm) {
            body = match fault {
                Fault::Duplicate => format!("eat({0}) + {1}", names[0], sum.join(" + ")),
                Fault::Drop => sum[..sum.len() - 1].join(" + "),
            };
        }
        text += &format!("        I{arm}({binders}) then {body}\n");
    }
    text
}

fn limits(work_max: u32) -> Limits {
    Limits {
        work_max,
        flow_work_max: 1_000_000_000,
        ..Limits::default()
    }
}

fn check(text: &str, work_max: u32) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict fixture");
    check_module("case.decoder", &source, &[], limits(work_max)).map(|_| ())
}

/// The smallest work budget under which `text` checks, or `None` above `ceiling`.
fn minimum_work(text: &str, ceiling: u32) -> Option<u32> {
    if check(text, ceiling).is_err() {
        return None;
    }
    let (mut low, mut high) = (0, ceiling);
    while low < high {
        let middle = low + (high - low) / 2;
        match check(text, middle) {
            Ok(()) => high = middle,
            Err(SourceCheckError::Budget) => low = middle + 1,
            Err(other) => panic!("{other:?}\n{text}"),
        }
    }
    Some(low)
}

#[test]
fn the_fixture_checks_and_each_arm_is_individually_reachable() {
    for arms in 1..=3 {
        check(&decoder(arms, 3, None), 200_000_000).unwrap();
    }
}

/// The cost model: each added arm adds a roughly constant amount, not a multiple of
/// everything before it. Arms that are not selected in a world are checked in the world that
/// selects them, not again inside every other arm.
#[test]
fn arm_cost_is_linear_in_the_number_of_arms() {
    let ceiling = 50_000_000;
    let cost: Vec<u32> = (1..=4)
        .map(|arms| {
            minimum_work(&decoder(arms, 3, None), ceiling)
                .unwrap_or_else(|| panic!("{arms} arms exceed {ceiling}"))
        })
        .collect();
    let step = |from: usize| cost[from + 1] - cost[from];
    // Allow the first arm to carry fixed costs; later arms must cost about the same each.
    assert!(step(2) <= 2 * step(1), "{cost:?}");
    assert!(step(1) <= 2 * step(0).max(step(1) / 2 + 1), "{cost:?}");
    assert!(cost[3] < 4 * cost[1], "{cost:?}");
}

/// Operands are worlds too (3^arity), but that is inherent: the cost for one arm must grow
/// by about the operand fan-out per added operand, never by more than the fan-out squared.
#[test]
fn operand_fan_out_is_the_only_exponential() {
    let one = minimum_work(&decoder(1, 1, None), 50_000_000).unwrap();
    let two = minimum_work(&decoder(1, 2, None), 50_000_000).unwrap();
    let three = minimum_work(&decoder(1, 3, None), 50_000_000).unwrap();
    assert!(one < two && two < three);
    assert!(three < 20 * one, "{one} {two} {three}");
}

#[test]
fn a_bad_last_arm_is_rejected_in_the_world_that_selects_it() {
    let text = decoder_with(3, 3, Some(2), Fault::Drop);
    assert_eq!(
        check(&text, 200_000_000).unwrap_err(),
        SourceCheckError::Unconsumed,
        "{text}"
    );
}

#[test]
fn a_bad_first_arm_is_rejected_too() {
    let text = decoder_with(3, 3, Some(0), Fault::Drop);
    assert_eq!(
        check(&text, 200_000_000).unwrap_err(),
        SourceCheckError::Unconsumed,
        "{text}"
    );
}

#[test]
fn a_bad_arm_that_moves_an_operand_twice_is_rejected() {
    for bad in [0, 2] {
        let text = decoder(3, 3, Some(bad));
        assert_eq!(
            check(&text, 200_000_000).unwrap_err(),
            SourceCheckError::UseAfterMove,
            "{text}"
        );
    }
}

/// The subject's member is fixed before the `when`: the other arms are never selected in
/// any world of this function, so only the hypothetical check can reject their bodies.
#[test]
fn an_arm_that_no_world_selects_is_still_checked() {
    let mut text = decoder(2, 3, None);
    text += "\nbuild() Instruction:\n    return I0(x: A, y: B, z: C)\n";
    // `known` never inspects `Instruction`'s member itself: it is constructed here.
    text += "known() in 0 to 6:\n    return when build() is\n";
    text += "        I0(x, y, z) then eat(x) + eat(y) + eat(z)\n";
    text += "        I1(x, y, z) then eat(x) + eat(y)\n";
    assert_eq!(
        check(&text, 200_000_000).unwrap_err(),
        SourceCheckError::Unconsumed,
        "{text}"
    );
}

#[test]
fn the_same_call_with_a_valid_unselected_arm_checks() {
    let mut text = decoder(2, 3, None);
    text += "\nbuild() Instruction:\n    return I0(x: A, y: B, z: C)\n";
    text += "known() in 0 to 6:\n    return when build() is\n";
    text += "        I0(x, y, z) then eat(x) + eat(y) + eat(z)\n";
    text += "        I1(x, y, z) then eat(x) + eat(y) + eat(z)\n";
    check(&text, 200_000_000).unwrap();
}
