mod support;

use internment::Intern;
use typecheck::DefId;

fn word_source(body: &str) -> String {
    format!(
        "{}\nWord is in -128 to 127\nWord has Numeric\n{body}",
        include_str!("../../../modules/core/ops/numeric.gin"),
    )
}

#[test]
fn conversion_preserves_the_integer_domain_and_allows_primitive_arithmetic() {
    for (method, receiver) in [("as_num", "ref "), ("to_num", "")] {
        let typed = support::check(&word_source(&format!(
            "number(value {receiver}Word): return value.{method}\nscaled(value {receiver}Word): return value.{method} * 2\n"
        )));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let result = &typed.defs[&DefId::named(Intern::from_ref("number"))].return_type;
        let validity = result
            .anonymous_integer_validity()
            .expect("anonymous integer result");
        assert!(validity.domain().contains((-128).into()));
        assert!(validity.domain().contains(127.into()));
        assert!(!validity.domain().contains(128.into()));
    }
}

#[test]
fn conversion_observes_a_generic_count_without_consuming_it() {
    let typed = support::check(&word_source(
        "#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has Numeric\nread(c Count(Word)):\n    first: c.as_num\n    second: c.as_num\n    last: c.to_num\n    return first + second + last\n",
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn trait_constrained_conversion_allows_primitive_arithmetic() {
    for (method, receiver) in [("as_num", "ref "), ("to_num", "")] {
        let typed = support::check(&word_source(&format!(
            "measure(value {receiver}has Numeric): return value.{method} * 2\nmeasured(value {receiver}Word): return measure(value)\n"
        )));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

#[test]
fn consuming_conversion_rejects_reusing_the_receiver() {
    let typed = support::check(&word_source(
        "read(value own Word):\n    first: value.to_num\n    second: value.to_num\n    return first + second\n",
    ));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-use-after-move"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn consuming_conversion_provides_a_terminal_exit() {
    let typed = support::check(&word_source("read(value own Word): return value.to_num\n"));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn inherited_numeric_defaults_reject_records_without_an_integer_domain() {
    let typed = support::check(&word_source("Record has Numeric\n    field Word\n"));
    assert!(
        support::flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-non-integer-domain"),
        "{:?}",
        typed.all_flaws()
    );
}
