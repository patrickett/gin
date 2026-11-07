//! A named domain not marked `#unrestricted` is its own linear type.
use typecheck::{
    AggregateKind, Capability, CheckedSourceFile, Limits, SourceCheckError, check_file,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict domain fixture");
    check_file(&file, Limits::default())
}

#[test]
fn an_unmarked_domain_is_a_nominal_type_with_a_hidden_integer() {
    let checked = check("FileId is in 0 to 255\nFileId has\n  close(self): return\n").unwrap();
    assert_eq!(checked.aggregates.len(), 1);
    assert_eq!(checked.aggregates[0].kind, AggregateKind::Domain);
    assert_eq!(checked.aggregates[0].capability, Capability::Linear);
    assert_eq!(checked.aggregates[0].fields.len(), 1);
}

#[test]
fn a_domain_accepts_a_has_block_of_methods() {
    let source = "FileId is in 0 to 255\nFileId has\n  close(self): return\n";
    let checked = check(source).unwrap();
    assert_eq!(checked.functions[0].name, "FileId.close");
}

#[test]
fn a_has_block_for_a_domain_holds_methods_only() {
    let source = "FileId is in 0 to 255\nFileId has\n  extra in 0 to 1\n";
    assert_eq!(check(source), Err(SourceCheckError::Unsupported));
}

const FILE: &str = "\
FileId is in 0 to 255
FileId has
  close(self): return
  to_int(self) in 0 to 255: self
";

fn with(tail: &str) -> String {
    format!("{FILE}{tail}")
}

#[test]
fn self_reads_as_an_integer_inside_the_domains_own_methods() {
    assert!(check(FILE).is_ok());
    let narrow = FILE.replace("in 0 to 255: self", "in 0 to 100: self");
    assert!(check(&narrow).is_err(), "255 does not fit 0 to 100");
}

#[test]
fn a_domain_never_reads_as_an_integer_outside_its_own_methods() {
    assert!(check(&with("peek(id FileId) in 0 to 255: id\n")).is_err());
    let other = "\
Count is in 0 to 255
Count has
  close(self): return
FileId is in 0 to 255
FileId has
  close(self): return
  steal(self, c Count) in 0 to 255: c
";
    assert!(check(other).is_err());
}

#[test]
fn an_integer_becomes_a_domain_only_when_its_interval_is_proved_inside() {
    for (tail, expected) in [
        ("make() FileId: return 7\n", None),
        ("make() FileId: return 255\n", None),
        ("make() FileId: return 300\n", Some(SourceCheckError::Type)),
        (
            "make() FileId: return 0 - 1\n",
            Some(SourceCheckError::Type),
        ),
        (
            "make(a in 0 to 300) FileId: return a\n",
            Some(SourceCheckError::Unproved),
        ),
        ("make(a in 0 to 200) FileId: return a + 5\n", None),
    ] {
        let result = check(&with(tail));
        match expected {
            None => assert!(result.is_ok(), "{tail}: {result:?}"),
            Some(error) => assert_eq!(result, Err(error), "{tail}"),
        }
    }
}

#[test]
fn a_domain_value_is_linear_and_its_methods_consume_it() {
    let closed = with("run():\n  id FileId: 7\n  id.close\n  return\n");
    assert!(check(&closed).is_ok());
    let leaked = with("run():\n  id FileId: 7\n  return\n");
    assert_eq!(check(&leaked), Err(SourceCheckError::Unconsumed));
    let twice = with("run():\n  id FileId: 7\n  id.close\n  id.close\n  return\n");
    assert_eq!(check(&twice), Err(SourceCheckError::UseAfterMove));
    let free = with("drop(id FileId): return\n");
    assert_eq!(check(&free), Err(SourceCheckError::Unconsumed));
}

#[test]
fn a_domain_value_may_be_passed_on_and_returned() {
    let source =
        with("pass(id FileId) FileId: return id\nrun():\n  id: pass(7)\n  id.close\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn domains_with_the_same_bounds_are_different_types() {
    let source = "\
FileId is in 0 to 255
FileId has
  close(self): return
Count is in 0 to 255
Count has
  close(self): return
take(c Count): c.close
give() FileId: return 1
run():
  id: give()
  take(id)
  return
";
    assert_eq!(check(source), Err(SourceCheckError::Type));
}

#[test]
fn a_linear_domain_must_define_a_terminal_method() {
    for (source, why) in [
        ("FileId is in 0 to 255\n", "no has block"),
        (
            "FileId is in 0 to 255\nFileId has\n  fresh() FileId: 1\n",
            "no method takes self",
        ),
        (
            "FileId is in 0 to 255\nFileId has\n  again(self) FileId: self\n",
            "the only self method hands the domain back",
        ),
    ] {
        assert_eq!(
            check(source),
            Err(SourceCheckError::MissingTerminal),
            "{why}"
        );
    }
}

#[test]
fn any_method_that_takes_self_and_does_not_return_the_domain_is_terminal() {
    let source = "FileId is in 0 to 255\nFileId has\n  value(self) in 0 to 255: self\n";
    assert!(check(source).is_ok());
}

#[test]
fn an_unrestricted_domain_is_nominal_copyable_and_needs_no_terminal() {
    let source = "#unrestricted\nSmall is in 0 to 255\nf(x Small) Small: return x\ng(x Small):\n  a: x\n  b: x\n  return\n";
    let checked = check(source).unwrap();
    assert_eq!(checked.aggregates.len(), 1);
    assert_eq!(checked.aggregates[0].kind, AggregateKind::Domain);
    assert_eq!(checked.aggregates[0].capability, Capability::Unrestricted);
}

#[test]
fn every_nominal_type_reports_its_kind_and_a_single_capability() {
    let source = "\
Full has v in 0 to 1
Slot is Empty or Full(Full)
FileId is in 0 to 255
FileId has
  close(self): return
";
    let checked = check(source).unwrap();
    let kinds: Vec<_> = checked
        .aggregates
        .iter()
        .map(|a| (a.name.as_str(), a.kind, a.capability))
        .collect();
    assert_eq!(
        kinds,
        [
            ("Full", AggregateKind::Record, Capability::Linear),
            ("Slot", AggregateKind::Sum, Capability::Linear),
            ("FileId", AggregateKind::Domain, Capability::Linear),
            // A union's own variants are recorded after the declared types.
            ("Slot.Empty", AggregateKind::Record, Capability::Unrestricted),
            ("Slot.Full", AggregateKind::Record, Capability::Linear),
        ]
    );
}
