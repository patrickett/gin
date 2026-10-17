mod support;

const PARCEL: &str =
    "Parcel has\n    weight in 0 to 100\n    priority in 0 to 1\n    finish(self): return\n";

#[test]
fn constructor_literals_must_belong_to_each_field_domain() {
    for (arguments, invalid) in [
        ("weight: 101, priority: 1", "101"),
        ("weight: -1, priority: 1", "-1"),
        ("weight: 100, priority: 2", "2"),
        ("priority: 2, weight: 100", "2"),
    ] {
        let source = format!("{PARCEL}make: return Parcel({arguments})\n");
        let typed = support::check_parsed(&source);
        let flaws = typed.all_flaws();
        let (_, diagnostic) = flaws
            .iter()
            .find(|(_, flaw)| flaw.code.slug() == "type-out-of-range")
            .unwrap_or_else(|| panic!("{source}\n{flaws:?}"));
        assert_eq!(diagnostic.arg("value"), Some(invalid));
        let span = diagnostic.span;
        assert_eq!(&source[span.start()..span.end()], invalid);
    }
}

#[test]
fn constructor_accepts_both_domain_boundaries() {
    for arguments in ["weight: 0, priority: 0", "priority: 1, weight: 100"] {
        support::check_no_flaws(&format!("{PARCEL}make: return Parcel({arguments})\n"));
    }
}

#[test]
fn constructor_parameters_must_be_contained_by_field_domains() {
    let source = format!("{PARCEL}make(weight in 0 to 101): return Parcel(weight, priority: 1)\n");
    let typed = support::check_parsed(&source);
    assert!(
        support::flaw_codes(&typed).contains(&"type-mismatch".to_string()),
        "{:?}",
        typed.all_flaws()
    );
    support::check_no_flaws(&format!(
        "{PARCEL}make(weight in 0 to 100): return Parcel(weight, priority: 1)\n"
    ));
}

#[test]
fn constructor_computed_arguments_must_belong_to_field_domains() {
    for expression in ["100 + 1", "0 - 1", "too_heavy"] {
        let source = format!(
            "{PARCEL}too_heavy: return 101\n\
             make: return Parcel(weight: {expression}, priority: 1)\n"
        );
        let typed = support::check_parsed(&source);
        assert!(
            !typed.all_flaws().is_empty(),
            "invalid constructor accepted:\n{source}"
        );
    }
    support::check_no_flaws(&format!(
        "{PARCEL}make: return Parcel(weight: 50 + 50, priority: 1)\n"
    ));
}

#[test]
fn constructor_respects_excluded_values_in_named_field_domains() {
    let prelude = concat!(
        "Nonzero is in 0 to 100 and /= 50\n",
        "Nonzero has finish(self): return\n",
        "Parcel has\n",
        "    weight Nonzero\n",
        "    finish(self): return\n",
    );
    let typed = support::check_parsed(&format!("{prelude}make: return Parcel(weight: 50)\n"));
    assert!(
        !typed.all_flaws().is_empty(),
        "excluded constructor value accepted"
    );
    for weight in [0, 49, 51, 100] {
        support::check_no_flaws(&format!("{prelude}make: return Parcel(weight: {weight})\n"));
    }
}
