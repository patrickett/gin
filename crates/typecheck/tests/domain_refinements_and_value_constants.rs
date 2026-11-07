//! A domain declaration may refine its range (`and self % 4096 = 0`): every conversion into the
//! domain must prove the refinement. And a module-level `name := expression` may hold any
//! compile-time value (records of text, numbers and other records, a choice by target).
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

const PAGE: &str = "top := 1048576\n#unrestricted\nPage is in 0 to top and self % 4096 = 0\n\n";

#[test]
fn a_literal_multiple_converts_into_the_domain() {
    check(&format!("{PAGE}ok() Page: return 8192 as Page\n")).unwrap();
}

#[test]
fn a_number_not_shown_to_be_a_multiple_is_refused() {
    for body in [
        "bad() Page: return 100 as Page\n",
        "bad(n in 0 to top) Page: return n as Page\n",
        "bad(n in 0 to top and n % 2048 = 0) Page: return n as Page\n",
    ] {
        assert_eq!(check(&format!("{PAGE}{body}")), Err(SourceCheckError::Unproved), "{body}");
    }
}

#[test]
fn a_parameter_that_claims_the_multiple_proves_it() {
    check(&format!("{PAGE}ok(n in 0 to top and n % 4096 = 0) Page: return n as Page\n")).unwrap();
    // A multiple of a multiple is a multiple.
    check(&format!("{PAGE}ok(n in 0 to top and n % 8192 = 0) Page: return n as Page\n")).unwrap();
}

#[test]
fn a_value_of_the_domain_carries_its_refinement() {
    check(&format!("{PAGE}again(p Page) Page: return (n in Page: p) as Page\n")).unwrap();
}

#[test]
fn comparison_refinements_are_settled_by_the_range() {
    let small = "#unrestricted\nSmall is in 0 to 100 and self <= 50\n\n";
    check(&format!("{small}ok(n in 0 to 50) Small: return n as Small\n")).unwrap();
    assert_eq!(
        check(&format!("{small}bad(n in 0 to 51) Small: return n as Small\n")),
        Err(SourceCheckError::Unproved)
    );
}

const RECORDS: &str = "\
#unrestricted
Reg has
    arch Text
    index in 0 to 255

#unrestricted
Desc has
    reg Reg
    target Text

";

#[test]
fn a_constant_may_be_a_record() {
    check(&format!(
        "{RECORDS}rax := Reg(arch: 'x86_64', index: 0)\ngo() in 0 to 255:\n    r: rax\n    return r.index\n"
    ))
    .unwrap();
}

#[test]
fn a_constant_may_be_a_record_of_records_chosen_by_target() {
    check(&format!(
        "{RECORDS}rax := Reg(arch: 'x86_64', index: 0)\npick := when #target.triple is\n    'x86_64-unknown-linux-gnu' then Desc(reg: rax, target: 'linux')\n    else Desc(reg: rax, target: 'other')\n\nis_linux() in 0 to 1:\n    d: pick\n    return when d.target is\n        'linux' then 1\n        else 0\n"
    ))
    .unwrap();
}

#[test]
fn integer_constants_are_still_numbers() {
    check("size := 4 * 1024\ngo() in 4096 to 4096: return size\n").unwrap();
}

#[test]
fn a_constant_of_the_wrong_shape_is_refused() {
    assert!(check(&format!("{RECORDS}bad := Reg(arch: 1, index: 0)\n")).is_err());
}
