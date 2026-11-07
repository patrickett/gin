//! `when left <op> right then A else B` over integers: each branch is checked under the
//! comparison (or its negation), and a branch the intervals rule out is not taken.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict conditional fixture");
    check_file(&file, Limits::default())
}

#[test]
fn a_conditional_chooses_between_two_values() {
    let source = "f(a in 0 to 9, b in 0 to 9) in 0 to 1:\n  return when a < b then 1 else 0\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn the_branch_taken_knows_the_condition_held() {
    // Inside the then-branch `a < 5`, so `a` fits `0 to 4`; without that it would not.
    let proved = "f(a in 0 to 9) in 0 to 4:\n  return when a < 5 then a else 0\n";
    assert!(check(proved).is_ok(), "{:?}", check(proved));
    let unproved = "f(a in 0 to 9) in 0 to 4:\n  return when a < 6 then a else 0\n";
    assert!(check(unproved).is_err());
}

#[test]
fn the_other_branch_knows_the_condition_failed() {
    let source = "f(a in 0 to 9) in 5 to 9:\n  return when a < 5 then 5 else a\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let wrong = "f(a in 0 to 9) in 6 to 9:\n  return when a < 5 then 6 else a\n";
    assert!(check(wrong).is_err());
}

#[test]
fn every_comparison_narrows_in_both_directions() {
    for (condition, narrowed) in [
        ("a < 5", "0 to 4"),
        ("a <= 5", "0 to 5"),
        ("a > 5", "6 to 9"),
        ("a >= 5", "5 to 9"),
    ] {
        let lower = narrowed.split(' ').next().unwrap();
        let source = format!(
            "f(a in 0 to 9) in {narrowed}:\n  return when {condition} then a else {lower}\n"
        );
        assert!(check(&source).is_ok(), "{condition}: {:?}", check(&source));
    }
}

#[test]
fn equality_narrows_the_taken_branch_to_one_value() {
    let source = "f(a in 0 to 9) in 0 to 3:\n  return when a = 3 then a else 0\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_branch_the_intervals_rule_out_is_not_taken() {
    let never_else = "f(a in 0 to 9) in 1 to 1:\n  return when a < 20 then 1 else 0\n";
    assert!(check(never_else).is_ok(), "{:?}", check(never_else));
    let never_then = "f(a in 0 to 9) in 1 to 1:\n  return when a > 20 then 5 else 1\n";
    assert!(check(never_then).is_ok(), "{:?}", check(never_then));
}

#[test]
fn both_branches_are_checked_against_the_result() {
    let source = "f(a in 0 to 9) in 0 to 1:\n  return when a < 5 then 1 else 2\n";
    assert!(check(source).is_err());
}

#[test]
fn conditionals_nest_and_chain() {
    let source =
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 3 then 0 else when a < 6 then 1 else 2\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let inner =
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 6 then when a < 3 then 0 else 1 else 2\n";
    assert!(check(inner).is_ok(), "{:?}", check(inner));
}

#[test]
fn only_a_comparison_of_integers_is_a_condition() {
    for source in [
        "f(a in 0 to 9, b in 0 to 9) in 0 to 1:\n  return when a + b then 1 else 0\n",
        "f(a in 0 to 9) in 0 to 1:\n  return when a then 1 else 0\n",
        "M is in 0 to 9\nM has\n  close(self): return\nf(a M, b M) in 0 to 1:\n  return when a < b then 1 else 0\n",
    ] {
        assert!(check(source).is_err(), "{source}");
    }
}

#[test]
fn ownership_is_accounted_in_every_world() {
    let both = "P has x in 0 to 9\nf(a in 0 to 9, p P) P:\n  return when a < 5 then p else p\n";
    assert!(check(both).is_ok(), "{:?}", check(both));
    let lost = "P has x in 0 to 9\nmake() P: return P(x: 1)\nf(a in 0 to 9, p P) P:\n  return when a < 5 then p else make()\n";
    assert_eq!(check(lost), Err(SourceCheckError::Unconsumed));
}

const TRUTH: &str = "\
#unrestricted
Truth is True or False
";

#[test]
fn a_library_can_write_a_real_ordering_with_a_conditional() {
    let source = format!(
        "{TRUTH}#unrestricted\nCount is in 0 to 9\nCount has\n  #infix('<')\n  less(self ref, rhs ref Self) Truth:\n    a in Self: self\n    b in Self: rhs\n    return when a < b then True else False\npick(x Count, y Count) in 0 to 1:\n  return when x < y is\n    True then 1\n    False then 0\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_library_can_write_equality_the_way_the_legacy_trait_did() {
    let source = format!(
        "{TRUTH}#unrestricted\nCount is in 0 to 9\nCount has\n  #infix('=')\n  equals(self ref, rhs ref Self) Truth:\n    a in Self: self\n    b in Self: rhs\n    return when a = b then True else False\n"
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

struct Bodies<'a>(&'a [Option<CheckedSourceBody>]);

impl BodyResolver for Bodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0[function as usize].as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn int(value: i64) -> Evaluated {
    Evaluated::Integer(I256::from(value))
}

fn run(source: &str, function: &str, arguments: &[Evaluated]) -> Option<Evaluated> {
    let checked = check(source).unwrap();
    let index = checked.functions.iter().position(|f| f.name == function)?;
    let body = checked.bodies[index].as_ref()?;
    evaluate_value(body, arguments, &Bodies(&checked.bodies), &mut 10_000)
}

#[test]
fn a_recorded_conditional_evaluates_to_the_branch_the_comparison_selects() {
    let max = "max(a in 0 to 9, b in 0 to 9) in 0 to 9: return when a < b then b else a\n";
    for (a, b, expected) in [(3, 7, 7), (7, 3, 7), (4, 4, 4), (0, 9, 9)] {
        assert_eq!(
            run(max, "max", &[int(a), int(b)]),
            Some(int(expected)),
            "{a} {b}"
        );
    }
}

#[test]
fn every_comparison_selects_correctly_when_evaluated() {
    for (op, cases) in [
        ("<", [(1, 2, true), (2, 2, false), (3, 2, false)]),
        ("<=", [(1, 2, true), (2, 2, true), (3, 2, false)]),
        (">", [(1, 2, false), (2, 2, false), (3, 2, true)]),
        (">=", [(1, 2, false), (2, 2, true), (3, 2, true)]),
        ("=", [(1, 2, false), (2, 2, true), (3, 2, false)]),
    ] {
        let source =
            format!("f(a in 0 to 9, b in 0 to 9) in 0 to 1: return when a {op} b then 1 else 0\n");
        for (a, b, holds) in cases {
            assert_eq!(
                run(&source, "f", &[int(a), int(b)]),
                Some(int(i64::from(holds))),
                "{a} {op} {b}"
            );
        }
    }
}

#[test]
fn a_chained_conditional_evaluates_each_range() {
    let source =
        "f(a in 0 to 9) in 0 to 2:\n  return when a < 3 then 0 else when a < 6 then 1 else 2\n";
    for (a, expected) in [(0, 0), (2, 0), (3, 1), (5, 1), (6, 2), (9, 2)] {
        assert_eq!(run(source, "f", &[int(a)]), Some(int(expected)), "{a}");
    }
}

#[test]
fn a_library_ordering_records_and_evaluates_to_true_or_false() {
    let source = format!(
        "{TRUTH}#unrestricted\nCount is in 0 to 9\nCount has\n  #infix('<')\n  less(self ref, rhs ref Self) Truth:\n    a in Self: self\n    b in Self: rhs\n    return when a < b then True else False\nbelow(x ref Count, y ref Count) Truth: return x < y\n"
    );
    let checked = check(&source).unwrap();
    let schema = |name: &str| {
        checked.aggregates.iter().position(|a| a.name == name).unwrap_or_else(|| {
            panic!("{name} in {:?}", checked.aggregates.iter().map(|a| &a.name).collect::<Vec<_>>())
        }) as u32
    };
    let count = |n: i64| Evaluated::Record {
        schema: schema("Count"),
        fields: vec![int(n)],
    };
    let truth = |holds: bool| Evaluated::Record {
        schema: schema(if holds { "Truth.True" } else { "Truth.False" }),
        fields: vec![],
    };
    assert_eq!(
        run(&source, "below", &[count(1), count(2)]),
        Some(truth(true))
    );
    assert_eq!(
        run(&source, "below", &[count(2), count(2)]),
        Some(truth(false))
    );
    assert_eq!(
        run(&source, "below", &[count(5), count(2)]),
        Some(truth(false))
    );
}

#[test]
fn an_unrestricted_domain_compares_as_its_number_in_a_condition() {
    let base = "#unrestricted\nLen is in 0 to 9\n";
    for body in [
        "f(count Len) in 0 to 1:\n  offset in Len: 0\n  when offset < count then\n    return 1\n  return 0\n",
        "f(count Len) in 0 to 9:\n  failure in 0 to 1: 0\n  offset in Len: 0\n  while failure = 0 and offset < count do\n    offset :: 9\n  return 0\n",
        "f(first Len) in 0 to 9:\n  lower in 0 to 9: when first = 4 then 1 else 2\n  return lower\n",
    ] {
        let source = format!("{base}{body}");
        assert!(check(&source).is_ok(), "{source}\n{:?}", check(&source));
    }
    // A linear domain is still not read in a condition (the existing control above).
}

#[test]
fn a_domain_keeps_what_an_earlier_condition_of_the_path_taught() {
    // Regression: after the path that chose `first = 224`, `first = 237` is impossible; the second
    // conditional used to see the domain's number without that and assumed both (a `Type`).
    let source = "#unrestricted\nByte is in 0 to 255\nf(first Byte) in 0 to 255:\n  lower Byte: when first = 224 then 160 else when first = 240 then 144 else 128\n  upper Byte: when first = 237 then 159 else when first = 244 then 143 else 191\n  return 0\n";
    assert!(check(source).is_ok(), "{:?}", check(source));
    // Negative control: the value of the second conditional is still checked against its type.
    let wrong = source.replace("then 159", "then 300");
    assert!(check(&wrong).is_err());
}
