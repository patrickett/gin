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
        ("[]", Ty::Sequence(Vec::new())),
        (
            "\"abc\"",
            Ty::Array {
                elem: Box::new(Ty::u8()),
                size: ast::NormalExpr::from(3_i128),
            },
        ),
    ] {
        let typed = check(&format!(
            "Box(value_type) has\n    value value_type\n    finish(self):\n        return\nbox := Box(value: {value})\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{value}: {:?}",
            typed.all_flaws()
        );
        let ty = &typed.defs[&DefId::named(Intern::from_ref("box"))].return_type;
        let Ty::Named { instance, .. } = ty else {
            panic!("{ty:?}")
        };
        assert_eq!(instance.arguments[0].1, TyArg::Type(Box::new(expected)));
    }
}

#[test]
fn target_property_can_be_stored_in_a_generic_record() {
    let typed = check(
        "Box(value_type) has\n    value value_type\n    finish(self):\n        return\nbox := Box(value: #target.triple)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn repeated_type_parameters_reject_conflicting_field_types() {
    let typed = check(
        "Pair(value_type) has left value_type, right value_type\npair := Pair(left: [], right: \"abc\")\n",
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

#[test]
fn bare_callable_parameters_are_type_parameters() {
    let typed = check("identity(element, value element) element: return value\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&DefId::named(Intern::from_ref("identity"))];
    assert!(matches!(bind.param_kinds[0], ast::ParamKind::Type));
    assert!(matches!(bind.param_kinds[1], ast::ParamKind::Value(_)));
}

#[test]
fn a_declared_type_named_type_is_an_ordinary_parameter_annotation() {
    let typed = check(
        "Type is in 0 to 255\nType has\n    finish(self):\n        return\nidentity(value Type) Type: return value\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&DefId::named(Intern::from_ref("identity"))];
    assert!(matches!(bind.param_kinds[0], ast::ParamKind::Value(_)));
}

#[test]
fn unresolved_type_annotation_does_not_create_a_type_parameter() {
    let typed = check("identity(value Type): return value\n");
    let bind = &typed.defs[&DefId::named(Intern::from_ref("identity"))];
    assert!(matches!(bind.param_kinds[0], ast::ParamKind::Value(_)));
}
