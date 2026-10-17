use super::{Case, Route, declarations};
use crate::integer_cases::{Domain, Integer, domains};
use ast::ConstValue;
use ginc::cli::Profile;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, FileId};

fn pairs_for(source: Domain) -> impl Iterator<Item = Case> {
    domains()
        .filter(move |target| !target.contains_domain(source))
        .map(move |target| Case {
            source,
            target,
            value: Integer::Nonnegative(0),
        })
}

fn boundary_cases(pair: Case) -> Vec<Case> {
    let (minimum, maximum) = pair.target.bounds();
    let mut values = pair.source.boundary_values();
    values.extend(pair.target.boundary_values());
    values.extend(minimum.predecessor());
    values.extend(maximum.successor());
    values.retain(|value| pair.source.contains(*value));
    values.sort_by(|left, right| left.compare(*right));
    values.dedup();
    values
        .into_iter()
        .map(|value| Case { value, ..pair })
        .collect()
}

fn definition(pair: Case, route: Route) -> String {
    let id = pair.symbol();
    let mut prefix = String::new();
    let subject = if route == Route::NamedCast {
        prefix.push_str(&format!("    named Source{id}: value as Source{id}\n"));
        "named"
    } else {
        "value"
    };
    let body = match route {
        Route::Local => format!(
            "        converted is in {}: value\n        return converted",
            pair.target
        ),
        Route::Parameter => format!("        return identity_{id}(value)"),
        Route::Return => "        return value".to_string(),
        Route::FieldRead => {
            return format!(
                "field_read_{id}(value is in {}) is in {}: return read_{id}(SourceBox{id}(number: value as Source{id}))\n",
                pair.source, pair.target
            );
        }
        Route::FieldWrite => format!(
            "        box: TargetBox{id}(number: value as Target{id})\n        raw is in {}: box.number\n        return raw",
            pair.target
        ),
        Route::AnonymousCast | Route::NamedCast => format!(
            "        converted Target{id}: {subject} as Target{id}\n        raw is in {}: converted\n        return raw",
            pair.target
        ),
    };
    format!(
        "{}_{id}(value is in {}) is in {}:\n{prefix}    when {subject} is in {} then\n{body}\n    return 0\n",
        route.symbol(),
        pair.source,
        pair.target,
        pair.target
    )
}

fn native_source(pair: Case, cases: &[Case], routes: &[Route]) -> String {
    let id = pair.symbol();
    let mut source = declarations(pair);
    source.push_str(&format!(
        "fits_{id}(value is in {}) True or False: return when value is in {} then True else False\n",
        pair.source, pair.target
    ));
    if routes.contains(&Route::Parameter) {
        source.push_str(&format!(
            "identity_{id}(input is in {}) is in {}: return input\n",
            pair.target, pair.target
        ));
    }
    if routes.contains(&Route::FieldRead) {
        source.push_str(&format!("read_{id}(box SourceBox{id}) is in {}:\n    when box.number is in {} then\n        converted Target{id}: box.number as Target{id}\n        raw is in {}: converted\n        return raw\n    return 0\n", pair.target, pair.target, pair.target));
    }
    for route in routes {
        source.push_str(&definition(pair, *route));
    }
    source.push_str("#entry\nmain:\n");
    for (index, case) in cases.iter().enumerate() {
        let accepted = case.target.contains(case.value);
        source.push_str(&format!(
            "    when fits_{id}({}) is {} then\n        return 1\n",
            case.value,
            if accepted { "False" } else { "True" }
        ));
        let expected = if accepted {
            case.value
        } else {
            Integer::Nonnegative(0)
        };
        for route in routes {
            source.push_str(&format!("    actual_{}_{index} is in {}: {}_{id}({})\n    when not (actual_{}_{index} = {expected}) then\n        return 1\n", route.symbol(), case.target, route.symbol(), case.value, route.symbol()));
        }
    }
    source.push_str("    return 0\n");
    source
}

#[test]
fn generated_native_guarded_narrowing_preserves_values_and_rejects_outside_values() {
    let mut checked = 0;
    let mut membership_checks = 0;
    let failure = |pair: Case, cases: &[Case], routes: &[Route], profile| {
        let source = native_source(pair, cases, routes);
        match crate::try_run_program(&source, "integer_narrowing_matrix", profile) {
            Ok(output) if output.status.success() => None,
            Ok(output) => Some(format!("{output:?}")),
            Err(error) => Some(error),
        }
    };
    for pair in domains().flat_map(pairs_for) {
        let cases = boundary_cases(pair);
        for profile in [Profile::Debug, Profile::Release] {
            if failure(pair, &cases, &Route::ALL, profile).is_none() {
                checked += cases.len() * Route::ALL.len();
                membership_checks += cases.len();
                continue;
            }
            for route in Route::ALL {
                if failure(pair, &cases, &[route], profile).is_none() {
                    continue;
                }
                for case in &cases {
                    if let Some(error) = failure(pair, &[*case], &[route], profile) {
                        panic!(
                            "{profile:?}, {case:?}, {route:?}: {error}\n{}",
                            native_source(pair, &[*case], &[route])
                        );
                    }
                }
                panic!("{profile:?}, {pair:?}, {route:?}: batch failed but isolated cases passed");
            }
            panic!("{profile:?}, {pair:?}: batch failed but isolated routes passed");
        }
    }
    assert_eq!(checked, 9_590);
    assert_eq!(membership_checks, 1_370);
}

#[test]
fn generated_constant_narrowing_preserves_in_range_values() {
    let mut checked = 0;
    for pair in domains().flat_map(pairs_for) {
        let mut source = declarations(pair);
        let id = pair.symbol();
        let cases: Vec<_> = boundary_cases(pair)
            .into_iter()
            .filter(|case| case.target.contains(case.value))
            .collect();
        for (index, case) in cases.iter().enumerate() {
            source.push_str(&format!("source_{index} is in {} := {}\nconverted_{index} := source_{index} as Target{id}\nnamed_{index} := source_{index} as Source{id}\nnamed_converted_{index} := named_{index} as Target{id}\n", pair.source, case.value));
        }
        let typed = transform(
            &TokenCursor::parse_source(&source),
            FileId(0),
            &TransformCtx::new(),
        );
        assert!(
            typed.all_flaws().is_empty(),
            "{pair:?}: {:?}\n{source}",
            typed.all_flaws()
        );
        for definition in typed.defs.values() {
            let Some(index) = ["converted_", "named_converted_"]
                .into_iter()
                .find_map(|prefix| definition.name.as_str().strip_prefix(prefix))
            else {
                continue;
            };
            let case = cases[index.parse::<usize>().unwrap()];
            let BindBody::Expr(body) = definition.body else {
                panic!("expected a constant conversion");
            };
            let evaluated = typecheck::Evaluator::new(
                typed.resolved_program(),
                typecheck::CompileTimeHost::new(&flask::CompileTarget::Library),
            )
            .eval(body)
            .unwrap();
            let typecheck::EvalOutcome::Complete(ConstValue::Int(value)) = evaluated else {
                panic!("{case:?}: {evaluated:?}");
            };
            assert_eq!(value.to_string(), case.value.to_string(), "{case:?}");
            checked += 1;
        }
    }
    assert_eq!(checked, 760);
}

#[test]
fn generated_unproven_and_out_of_range_narrowing_is_rejected() {
    let mut unproven = 0;
    let mut outside = 0;
    for pair in domains().flat_map(pairs_for) {
        let id = pair.symbol();
        for (parameter, expression) in [
            (format!("is in {}", pair.source), "value".to_string()),
            (format!("Source{id}"), "value".to_string()),
        ] {
            let source = format!(
                "{}convert(value {parameter}) Target{id}: return {expression} as Target{id}\n",
                declarations(pair)
            );
            let typed = transform(
                &TokenCursor::parse_source(&source),
                FileId(0),
                &TransformCtx::new(),
            );
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug().starts_with("type-integer-narrowing-")),
                "{pair:?}: {:?}\n{source}",
                typed.all_flaws()
            );
            unproven += 1;
        }
        for case in boundary_cases(pair)
            .into_iter()
            .filter(|case| !case.target.contains(case.value))
        {
            let source = format!(
                "{}source is in {} := {}\nconverted := source as Target{id}\n",
                declarations(pair),
                pair.source,
                case.value
            );
            let typed = transform(
                &TokenCursor::parse_source(&source),
                FileId(0),
                &TransformCtx::new(),
            );
            assert!(
                typed
                    .all_flaws()
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-integer-narrowing-failed"),
                "{case:?}: {:?}\n{source}",
                typed.all_flaws()
            );
            outside += 1;
        }
    }
    assert_eq!(unproven, 120);
    assert_eq!(outside, 305);
}

#[test]
fn guarded_narrowing_preserves_exclusions_at_runtime() {
    let mut source = "Nonzero is in -128 to 127 and /= 0\nNonzero has\n    finish(self):\n        return\nconvert(value is in -32768 to 32767) Nonzero: return when value is in -128 to 127 and not (value = 0) then value as Nonzero else 1\nconvert_tag(value is in -32768 to 32767) Nonzero: return when value is in Nonzero then value as Nonzero else 1\n#entry\nmain is in 0 to 255:\n".to_string();
    for (index, (value, expected)) in [
        (-129, 1),
        (-128, -128),
        (-127, -127),
        (-1, -1),
        (0, 1),
        (1, 1),
        (126, 126),
        (127, 127),
        (128, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let direct_failure = 2 * index + 1;
        let tagged_failure = direct_failure + 1;
        source.push_str(&format!("    actual_{index} is in -128 to 127: convert({value})\n    when not (actual_{index} = {expected}) then\n        return {direct_failure}\n"));
        source.push_str(&format!("    tagged_{index} is in -128 to 127: convert_tag({value})\n    when not (tagged_{index} = {expected}) then\n        return {tagged_failure}\n"));
    }
    source.push_str("    return 0\n");
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(&source, "narrowing_exclusions", profile);
    }
}

#[test]
fn guarded_narrowing_checks_full_unsigned_128_bit_bounds() {
    let maximum = u128::MAX;
    let midpoint = 1_u128 << 127;
    let mut source = format!(
        "Upper is in {midpoint} to {maximum}\nUpper has\n    finish(self):\n        return\nfits(value is in 0 to {maximum}) True or False: return when value is in Upper then True else False\nconvert(value is in 0 to {maximum}) Upper: return when value is in Upper then value as Upper else {midpoint}\n#entry\nmain:\n"
    );
    for (index, value) in [
        0,
        1,
        midpoint - 1,
        midpoint,
        midpoint + 1,
        maximum - 1,
        maximum,
    ]
    .into_iter()
    .enumerate()
    {
        let accepted = value >= midpoint;
        let expected = if accepted { value } else { midpoint };
        source.push_str(&format!("    when fits({value}) is {} then\n        return 1\n    actual_{index} is in {midpoint} to {maximum}: convert({value})\n    when not (actual_{index} = {expected}) then\n        return 1\n", if accepted { "False" } else { "True" }));
    }
    source.push_str("    return 0\n");
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(&source, "narrowing_unsigned_128_bounds", profile);
    }
}

#[test]
fn constant_narrowing_uses_complete_named_domain_patterns() {
    let source = "Nonzero is in -128 to 127 and /= 0\nNonzero has\n    finish(self):\n        return\nconvert(value is in -32768 to 32767) Nonzero: return when value is in Nonzero then value as Nonzero else 1\nconvert_match(value is in -32768 to 32767) Nonzero: return when value is\n    in Nonzero then value as Nonzero\n    else 1\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    for name in ["convert", "convert_match"] {
        let definition = *typed
            .defs
            .iter()
            .find(|(_, definition)| definition.name.as_str() == name)
            .unwrap()
            .0;
        for value in [-129_i128, -128, -127, -1, 0, 1, 126, 127, 128] {
            let expected = if (-128..=127).contains(&value) && value != 0 {
                value
            } else {
                1
            };
            let mut evaluator = typecheck::Evaluator::new(
                typed.resolved_program(),
                typecheck::CompileTimeHost::new(&target),
            );
            let actual = evaluator
                .call(definition, &[ConstValue::Int(value.into())])
                .unwrap();
            assert_eq!(actual, ConstValue::Int(expected.into()), "{name}({value})");
        }
    }
}

#[test]
fn guarded_conditional_returns_narrow_each_arm_at_runtime() {
    let mut source = "convert(value is in -32768 to 32767) is in 0 to 255: return when value is in 0 to 255 then value else 0\n#entry\nmain is in 0 to 255:\n".to_string();
    for (index, value) in [-32768, -1, 0, 1, 254, 255, 256, 32767]
        .into_iter()
        .enumerate()
    {
        let expected = if (0..=255).contains(&value) { value } else { 0 };
        source.push_str(&format!("    actual_{index} is in 0 to 255: convert({value})\n    when not (actual_{index} = {expected}) then\n        return {}\n", index + 1));
    }
    source.push_str("    return 0\n");
    for profile in [Profile::Debug, Profile::Release] {
        crate::assert_program(&source, "narrowing_conditional_returns", profile);
    }
}
