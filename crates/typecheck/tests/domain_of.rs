mod support;

use ast::ConstValue;
use internment::Intern;
use parser::query::SourceParseExt;
use support::check_parsed;
use typecheck::{BindBody, DefId, TypedFileAst};

fn value(typed: &TypedFileAst, name: &str) -> ConstValue {
    let bind = &typed.defs[&DefId::named(Intern::from_ref(name))];
    let BindBody::Expr(id) = bind.body else {
        panic!("expected expression body")
    };
    typed
        .exprs
        .const_value_of(id)
        .unwrap()
        .clone()
        .expect("constant")
}

const INTERVAL: &str =
    "Interval(lo to hi) is lo to hi\nInterval(lo to hi) has\n    lower: lo\n    upper: hi\n";

#[test]
fn core_range_exposes_both_endpoints_without_discarding_constraints() {
    let range = include_str!("../../../modules/core/range/range.gin");
    let typed = check_parsed(&format!(
        "{range}\nByte is in 0 to 255\nByte has finish(self): return\n\
         low: Range(domain of Byte).min\nhigh: Range(domain of Byte).max\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int(0.into()));
    assert_eq!(value(&typed, "high"), ConstValue::Int(255.into()));
    for source in [
        concat!(
            "Value is in 0 to 255 and /= 127\nValue has finish(self): return\n",
            "high: Range(domain of Value).max\n",
        ),
        "probe:\n    value is >= 0: 0\n    return Range(domain of value).max\n",
        "probe: Range(0).max\n",
    ] {
        let typed = check_parsed(&format!("{range}\n{source}"));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| { flaw.code.slug() == "type-range-pattern" }),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn library_range_patterns_expose_bounds_without_builtin_member_names() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in -128 to 127\nWord has\n    finish(self):\n        return\nlow: Interval(domain of Word).lower\nhigh: Interval(domain of Word).upper\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int((-128).into()));
    assert_eq!(value(&typed, "high"), ConstValue::Int(127.into()));
}

#[test]
fn domain_values_preserve_exclusions() {
    let typed = check_parsed(
        "NonZero is in -10 to 10 and /= 0\nNonZero has\n    finish(self):\n        return\nvalues: domain of NonZero\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let ConstValue::Domain(domain) = value(&typed, "values") else {
        panic!("domain")
    };
    assert!(domain.contains((-10).into()));
    assert!(domain.contains(10.into()));
    assert!(!domain.contains(0.into()));
}

#[test]
fn interval_patterns_reject_domains_with_holes() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nNonZero is in -10 to 10 and /= 0\nNonZero has\n    finish(self):\n        return\nlow: Interval(domain of NonZero).lower\n"
    ));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, d)| d.code.slug() == "type-range-pattern"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn self_domain_is_available_in_integer_methods() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in -128 to 127\nWord has\n    finish(self):\n        return\n    lower(): return Interval(domain of Self).lower\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "Word.lower"), ConstValue::Int((-128).into()));
}

#[test]
fn range_aliases_and_literal_ranges_use_the_same_properties() {
    let typed = check_parsed(&format!(
        "{INTERVAL}Small is Interval(-4 to 9)\nlow: Small.lower\nhigh: Interval(3 to 7).upper\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int((-4).into()));
    assert_eq!(value(&typed, "high"), ConstValue::Int(7.into()));
}

#[test]
fn separate_files_share_library_range_declarations() {
    let sources = [
        INTERVAL,
        "Word is in Interval(-32768 to 32767)\nWord has\n    finish(self):\n        return\nlow: Interval(domain of Word).lower\n",
    ];
    let files = sources
        .into_iter()
        .map(|source| {
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            parsed.ast
        })
        .collect();
    let package = typecheck::transform::transform_package_with_shared_context(
        files,
        typecheck::transform::PackageTransformOptions::FULL,
    );
    let typed = &package.typed_asts[1];
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(typed, "low"), ConstValue::Int((-32768).into()));
}

#[test]
fn non_integer_domain_operands_are_rejected() {
    let typed = check_parsed("Record has item\nvalue: domain of Record\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, d)| d.code.slug() == "type-domain-operand"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn library_range_can_define_a_nominal_integer_domain() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in Interval(-128 to 127)\nWord has\n    finish(self):\n        return\nlow: Interval(domain of Word).lower\nidentity(value Word) Word: return value\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int((-128).into()));
    let ty = &typed.defs[&DefId::named(Intern::from_ref("identity"))].return_type;
    assert!(ty.type_id().is_some());
}

#[test]
fn stored_domains_can_be_passed_to_library_range_patterns() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in -12 to 23\nWord has\n    finish(self):\n        return\nvalues: domain of Word\nlow: Interval(values).lower\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int((-12).into()));
}

#[test]
fn value_domain_uses_declared_parameter_type() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in -12 to 23\nWord has\n    finish(self):\n        return\nextract(value Word): return Interval(domain of value).lower\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "extract"), ConstValue::Int((-12).into()));
}

#[test]
fn range_extension_parameters_can_rename_endpoint_bindings() {
    let typed = check_parsed(
        "Interval(lo to hi) is lo to hi\nInterval(a to b) has\n    lower: a\n    upper: b\nlow: Interval(-4 to 7).lower\nhigh: Interval(-4 to 7).upper\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(value(&typed, "low"), ConstValue::Int((-4).into()));
    assert_eq!(value(&typed, "high"), ConstValue::Int(7.into()));
}

#[test]
fn integer_domains_defined_by_library_ranges_keep_nominal_identity_in_unions() {
    let typed = check_parsed(&format!(
        "{INTERVAL}\nWord is in Interval(-128 to 127)\nWord has\n    finish(self):\n        return\nAttempt is Word or Flaw\nAttempt has\n    finish(self):\n        return\naccept(value Word) Attempt: return value\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn domain_evaluation_does_not_hide_invalid_value_projections() {
    let typed = check_parsed("value: 4\nresult: value.missing\n");
    assert!(!typed.all_flaws().is_empty());
}
