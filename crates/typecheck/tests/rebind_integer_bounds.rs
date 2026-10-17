mod support;

#[test]
fn reassignments_check_the_replacements_lower_bound() {
    let typed = support::check(
        "inspect(value is in 1 to 10):\n    candidate is in 1 to 10: value\n    candidate :: candidate - 2\n    return candidate\n",
    );
    assert!(support::flaw_codes(&typed).contains(&"type-rebind-violates-validity".to_owned()));
}

#[test]
fn a_strict_upper_guard_allows_the_last_valid_increment() {
    let typed = support::check(
        "inspect(value is in 1 to 10):\n    candidate is in 1 to 10: value\n    when candidate < 10 then\n        candidate +: 1\n        return candidate\n    return candidate\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_strict_upper_guard_does_not_authorize_arbitrary_doubling() {
    let typed = support::check(
        "inspect(value is in 1 to 10):\n    candidate is in 1 to 10: value\n    when candidate < 10 then\n        candidate *: 2\n        return candidate\n    return candidate\n",
    );
    assert!(support::flaw_codes(&typed).contains(&"type-rebind-violates-validity".to_owned()));
}

#[test]
fn a_divisor_loop_keeps_its_declared_integer_bounds() {
    use parser::query::SourceParseExt;

    let parsed = "maximum := 16\ninspect(bytes is in 1 to maximum):\n    candidate is in 1 to maximum and maximum % candidate = 0: 1\n    while candidate < bytes do\n        candidate *: 2\n    return candidate\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut asts = vec![parsed.ast];
    let preparation_flaws =
        typecheck::prepare_package_asts(&mut asts, &flask::CompileTarget::Library);
    assert!(
        preparation_flaws.iter().all(Vec::is_empty),
        "{preparation_flaws:?}"
    );
    let typed = typecheck::transform::transform_file(asts.pop().unwrap(), typecheck::FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

fn prepared(source: &str) -> typecheck::TypedFileAst {
    use parser::query::SourceParseExt;

    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut asts = vec![parsed.ast];
    let flaws = typecheck::prepare_package_asts(&mut asts, &flask::CompileTarget::Library);
    assert!(flaws.iter().all(Vec::is_empty), "{flaws:?}");
    typecheck::transform::transform_file(asts.pop().unwrap(), typecheck::FileId(0))
}

#[test]
fn replacements_must_preserve_divisibility() {
    for (maximum, replacement, valid) in
        [(16, 3, false), (16, 4, true), (12, 8, false), (12, 4, true)]
    {
        let typed = prepared(&format!(
            "maximum := {maximum}\ninspect:\n    candidate is in 1 to maximum and maximum % candidate = 0: 1\n    candidate :: {replacement}\n    return candidate\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(
                codes.is_empty(),
                "maximum={maximum}, replacement={replacement}: {:?}",
                typed.all_flaws()
            );
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code == "type-rebind-violates-validity"),
                "invalid divisor accepted: maximum={maximum}, replacement={replacement}, {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn replacements_must_preserve_excluded_values() {
    for (replacement, valid) in [(3, false), (4, true)] {
        let typed = prepared(&format!(
            "inspect:\n    candidate is in 1 to 16 and /= 3: 1\n    candidate :: {replacement}\n    candidate\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(codes.is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code == "type-rebind-violates-validity"),
                "excluded value accepted: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn doubling_does_not_preserve_an_arbitrary_divisor_domain() {
    let typed = prepared(
        "maximum := 12\ninspect(bytes is in 1 to maximum):\n    candidate is in 1 to maximum and maximum % candidate = 0: 1\n    while candidate < bytes do\n        candidate *: 2\n    return candidate\n",
    );
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-rebind-violates-validity"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn a_runtime_replacement_needs_its_own_divisibility_proof() {
    for (refinement, valid) in [("", false), (" and maximum % next = 0", true)] {
        let typed = prepared(&format!(
            "maximum := 16\ninspect(next is in 1 to maximum{refinement}):\n    candidate is in 1 to maximum and maximum % candidate = 0: 1\n    candidate :: next\n    return candidate\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(codes.is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code == "type-rebind-violates-validity"),
                "unproved divisor accepted: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn a_guard_can_prove_that_a_replacement_avoids_an_excluded_value() {
    let typed = prepared(
        "inspect(next is in 1 to 16):\n    candidate is in 1 to 16 and /= 3: 1\n    when next < 3 then\n        candidate :: next\n        return candidate\n    return candidate\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn mutable_integer_parameters_preserve_their_pointees_validity() {
    for (replacement, valid) in [(3, false), (4, true)] {
        let typed = prepared(&format!(
            "maximum := 16\nDivisor is in 1 to maximum and maximum % self = 0\nDivisor has\n    finish(self):\n        return\nchange(value mut Divisor):\n    value :: {replacement}\n    return\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(codes.is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code == "type-rebind-violates-validity"),
                "invalid pointee accepted: {:?}",
                typed.all_flaws()
            );
        }
    }
}

#[test]
fn mutable_anonymous_integer_parameters_keep_their_bounds() {
    for (replacement, valid) in [(0, false), (4, true)] {
        let typed = prepared(&format!(
            "change(value mut is in 1 to 16):\n    value :: {replacement}\n    return\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(codes.is_empty(), "{:?}", typed.all_flaws());
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code == "type-rebind-violates-validity"),
                "out-of-bounds pointee accepted: {:?}",
                typed.all_flaws()
            );
        }
    }
}
