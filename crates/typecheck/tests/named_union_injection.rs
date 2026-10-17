mod support;

use ast::ConstValue;
use internment::Intern;
use typecheck::DefId;
use typecheck::eval::{CompileTimeHost, Evaluator};

const TYPES: &str = concat!(
    "Byte is in 0 to 255\nByte has finish(self): return\n",
    "MaybeByte is Byte or Missing\nMaybeByte has finish(self): return\n",
);

#[test]
fn a_named_member_is_injected_into_its_named_union() {
    let typed = support::check_no_flaws(&format!(
        "{TYPES}wrap(value Byte) MaybeByte: return value\n"
    ));
    let target = flask::CompileTarget::Library;
    let result = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target))
        .call(
            DefId::named(Intern::from_ref("wrap")),
            &[ConstValue::Int(7.into())],
        )
        .unwrap();
    assert!(matches!(
        result,
        ConstValue::Tag { name, args, .. }
            if name.as_str() == "Byte" && args.as_ref() == [ConstValue::Int(7.into())]
    ));
}

#[test]
fn imported_named_members_keep_their_union_injection() {
    let flaws = support::package_flaws(
        "named_results",
        &[
            ("types", TYPES),
            ("client", "wrap(value Byte) MaybeByte: return value\n"),
        ],
    );
    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn equal_integer_domains_do_not_make_an_unrelated_nominal_type_a_member() {
    let source = format!(
        "{TYPES}Other is in 0 to 255\nOther has finish(self): return\n\
         wrap(value Other) MaybeByte: return value\n"
    );
    let typed = support::check_parsed(&source);
    assert!(
        support::flaw_codes(&typed).contains(&"type-return-mismatch".to_owned()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn a_generic_record_field_can_infer_a_union_type() {
    support::check_no_flaws(&format!(
        "{TYPES}Box(element) has value element\n\
         pack(value MaybeByte): return Box(value: value)\n"
    ));
}

#[test]
fn a_concrete_record_field_still_rejects_an_incompatible_union() {
    let typed = support::check_parsed(&format!(
        "{TYPES}Box(element) has value element\n\
         pack(value MaybeByte) Box(Byte): return Box(value: value)\n"
    ));
    assert!(
        support::flaw_codes(&typed).iter().any(|code| {
            matches!(
                code.as_str(),
                "type-incompatible-union-value" | "type-return-mismatch"
            )
        }),
        "{:?}",
        typed.all_flaws()
    );
}
