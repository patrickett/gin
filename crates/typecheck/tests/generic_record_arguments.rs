use ast::{Ty, TyArg};
use flask::{CompileTarget, TargetTriple};
use internment::Intern;
use parser::parse_from_str;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{DefId, FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let mut context = TransformCtx::new();
    context.compile_target =
        CompileTarget::Concrete(TargetTriple::parse("aarch64-apple-darwin").unwrap());
    transform(&parsed, FileId(0), &context)
}

#[test]
fn constructor_infers_type_arguments_from_ordinary_values() {
    for (value, expected) in [
        ("()", Ty::Unit),
        (
            "\"abc\"",
            Ty::Array {
                elem: Box::new(Ty::u8()),
                size: ast::NormalExpr::from(3_i128),
            },
        ),
    ] {
        let typed = check(&format!(
            "Box(value_type Type) has value value_type\nbox := Box(value: {value})\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{value}: {:?}",
            typed.all_flaws()
        );
        let ty = &typed.defs[&DefId(Intern::from_ref("box"))].return_type;
        let Ty::Named { instance, .. } = ty else {
            panic!("{ty:?}")
        };
        assert_eq!(instance.arguments[0].1, TyArg::Type(Box::new(expected)));
    }
}

#[test]
fn target_property_can_be_stored_in_a_generic_record() {
    let typed =
        check("Box(value_type Type) has value value_type\nbox := Box(value: #target.triple)\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn repeated_type_parameters_reject_conflicting_field_types() {
    let typed = check(
        "Pair(value_type Type) has left value_type, right value_type\npair := Pair(left: (), right: \"abc\")\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-record-type-arguments"),
        "{:?}",
        typed.all_flaws()
    );
}
