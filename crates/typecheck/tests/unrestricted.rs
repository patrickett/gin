//! `#unrestricted` marks a domain or record whose values may be copied and discarded.
use typecheck::{Capability, CheckedSourceFile, Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict unrestricted fixture");
    check_file(&file, Limits::default())
}

const POINT: &str = "Point has x in 0 to 9, y in 0 to 9\n";
const COPY: &str = "f(p Point) Point:\n  q: p\n  r: p\n  return p\n";
const DISCARD: &str = "f(p Point):\n  return\n";

fn point(marker: &str, body: &str) -> String {
    format!("{marker}{POINT}{body}")
}

#[test]
fn domains_accept_the_marker_alone_and_with_wrap_in_either_order() {
    for source in [
        "#unrestricted\nSmall is in 0 to 255\n",
        "#unrestricted, #wrap\nSmall is in 0 to 255\n",
        "#wrap\n#unrestricted\nSmall is in 0 to 255\n",
        "#unrestricted\n#wrap\nSmall is in 0 to 255\n",
    ] {
        assert!(
            check(source).is_ok(),
            "{source:?}: {:?}",
            check(source).err()
        );
    }
}

#[test]
fn the_marker_only_marks_a_domain_record_or_sum() {
    for source in ["#unrestricted\nf(x in 0 to 9): return\n", "#unrestricted\n"] {
        assert_eq!(
            check(source),
            Err(SourceCheckError::UnrestrictedTarget),
            "{source:?}"
        );
    }
    assert_eq!(
        check("#unrestricted\n#unrestricted\nSmall is in 0 to 255\n"),
        Err(SourceCheckError::Duplicate)
    );
}

#[test]
fn the_classification_is_reported_on_the_checked_aggregate() {
    let plain = check(&point("", "")).unwrap();
    let marked = check(&point("#unrestricted\n", "")).unwrap();
    assert_eq!(plain.aggregates[0].capability, Capability::Linear);
    assert_eq!(marked.aggregates[0].capability, Capability::Unrestricted);
}

#[test]
fn records_stay_linear_unless_marked() {
    assert_eq!(
        check(&point("", DISCARD)),
        Err(SourceCheckError::Unconsumed)
    );
    assert_eq!(check(&point("", COPY)), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn an_unrestricted_record_may_be_discarded_and_copied() {
    let r = check(&point("#unrestricted\n", DISCARD));
    assert!(r.is_ok(), "{:?}", r.err());
    assert!(check(&point("#unrestricted\n", COPY)).is_ok());
}

#[test]
fn unrestricted_records_may_only_hold_unrestricted_fields() {
    let nested = |inner: &str, outer: &str| {
        format!("{inner}Inner has v in 0 to 9\n{outer}Outer has inner Inner\n")
    };
    assert!(check(&nested("#unrestricted\n", "#unrestricted\n")).is_ok());
    assert_eq!(
        check(&nested("", "#unrestricted\n")),
        Err(SourceCheckError::UnrestrictedField)
    );
    // An instance of a generic record is never unrestricted yet.
    assert_eq!(
        check(
            "Box(element) has item element\n#unrestricted\nHolder has box Box(Byte)\n#unrestricted\nByte is in 0 to 255\n"
        ),
        Err(SourceCheckError::UnrestrictedField)
    );
}

#[test]
fn a_linear_record_may_still_contain_unrestricted_ones() {
    let text =
        "#unrestricted\nInner has v in 0 to 9\nOuter has inner Inner\nf(o Outer):\n  return\n";
    assert_eq!(check(text), Err(SourceCheckError::Unconsumed));
}

const TAG: &str = "#unrestricted\nTag has v in 0 to 9\n";
const RES: &str = "Res has v in 0 to 9\n";
const BOX: &str = "#unrestricted\nBox(element) has item element\n";

#[test]
fn a_sum_is_unrestricted_only_when_every_variant_payload_is() {
    let sum = |variants: &str| format!("{variants}#unrestricted\nSlot is Left(Left) or Right(Right)\n");
    let ok = sum("#unrestricted\nLeft has v in 0 to 1\n#unrestricted\nRight has v in 0 to 1\n");
    assert!(check(&ok).is_ok(), "{:?}", check(&ok));
}

#[test]
fn a_linear_variant_payload_makes_an_unrestricted_sum_unrestricted_field() {
    let linear = "#unrestricted\nLeft has v in 0 to 1\nRight has v in 0 to 1\n#unrestricted\nSlot is Left(Left) or Right(Right)\n";
    assert_eq!(check(linear), Err(SourceCheckError::UnrestrictedField));
}

#[test]
fn an_unrestricted_generic_accepts_unrestricted_arguments() {
    let source = format!("{BOX}{TAG}mk(t Tag) Box(Tag): return Box(item: t)\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let copied = format!("{BOX}{TAG}twice(b Box(Tag)):\n  a: b\n  c: b\n  return\n");
    assert!(check(&copied).is_ok(), "{:?}", check(&copied));
}

#[test]
fn a_linear_type_cannot_be_passed_to_an_unrestricted_generic() {
    for tail in [
        "use_it(b Box(Res)): return\n",
        "Holder has box Box(Res)\n",
        "mk(r Res) Box(Res): return Box(item: r)\n",
    ] {
        let source = format!("{BOX}{RES}{tail}");
        assert_eq!(
            check(&source),
            Err(SourceCheckError::UnrestrictedArgument),
            "{tail}"
        );
    }
}

#[test]
fn an_unmarked_generic_stays_linear_for_every_argument() {
    let source = format!("{TAG}Box(element) has item element\nuse_it(b Box(Tag)): return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unconsumed));
}

#[test]
fn a_union_of_payload_less_variants_is_linear_unless_marked() {
    let body = "eat(r Reg) in 0 to 1: return when r is\n    A then 0\n    B then 1\n";
    // Without the attribute: used twice is a use after move, unused is `Unconsumed`.
    let plain = format!("Reg is A or B\n{body}twice(x Reg) in 0 to 2: return eat(x) + eat(x)\n");
    assert_eq!(check(&plain), Err(SourceCheckError::UseAfterMove));
    let unused = format!("Reg is A or B\n{body}unused(x Reg): return\n");
    assert_eq!(check(&unused), Err(SourceCheckError::Unconsumed));
    // With it: copyable and droppable.
    let marked = format!("#unrestricted\nReg is A or B\n{body}twice(x Reg) in 0 to 2: return eat(x) + eat(x)\nunused(x Reg): return\n");
    assert!(check(&marked).is_ok(), "{:?}", check(&marked));
    // A discriminant union follows the same rule.
    let coded = "Level is Low = 1 or High = 2\npass(l Level): return\n";
    assert_eq!(check(coded), Err(SourceCheckError::Unconsumed));
    assert!(check(&format!("#unrestricted\n{coded}")).is_ok());
}
