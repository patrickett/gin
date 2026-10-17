mod support;

use ast::{ConstValue, Ty};
use internment::Intern;
use parser::cursor::TokenCursor;

#[test]
fn unresolved_int_has_no_integer_representation() {
    assert!(
        typecheck::representation::Repr::derive(&Ty::Opaque(Intern::from_ref("Int")), None)
            .is_err()
    );
}

#[test]
fn comparison_does_not_resolve_a_library_bool_name() {
    use std::collections::HashMap;
    use typecheck::analysis::{TyInfer, TyInferEnv};
    let parsed = TokenCursor::parse_source("compare: 1 = 2\n");
    let ast::BindValue::Expr(expression) = &parsed.defs[&Intern::from_ref("compare")].value else {
        panic!()
    };
    let types = HashMap::from([(Intern::from_ref("Bool"), Ty::u8())]);
    let empty = HashMap::new();
    let env = TyInferEnv {
        tag_types: &types,
        fn_return_types: &empty,
        locals: &empty,
        tag_params: None,
    };
    assert!(matches!(expression.infer_ty(&env), Ty::Proposition(_)));
}

#[test]
fn ordinary_helper_names_evaluate_their_declared_bodies() {
    use typecheck::analysis::{CompTimeEvaluator, ConstEnv};
    for name in ["add", "sub", "mul", "max_size"] {
        let parsed =
            TokenCursor::parse_source(&format!("{name}(a, b): return 42\nresult: {name}(1, 2)\n"));
        let ast::BindValue::Expr(expression) = &parsed.defs[&Intern::from_ref("result")].value
        else {
            panic!()
        };
        assert_eq!(
            CompTimeEvaluator::new(&ConstEnv::default(), &parsed).eval(&expression.value),
            Some(ConstValue::Int(42.into())),
            "{name}"
        );
    }
}

#[test]
fn ordinary_const_and_dynamic_tags_are_not_numbers() {
    for name in ["Const", "Dynamic", "True", "False"] {
        let value = ConstValue::Tag {
            name: Intern::from_ref(name),
            qual_path: None,
            args: vec![ConstValue::Int(3.into())].into(),
        };
        assert_eq!(value.as_const_size_int(), None);
        assert_eq!(value.eval_binop(&ast::BinOp::Add, &value), None);
        assert_eq!(value.eval_binop(&ast::BinOp::LogicalAnd, &value), None);
    }
}

#[test]
fn list_name_and_pointer_field_do_not_imply_sequence_semantics() {
    let ty = Ty::Record {
        name: Intern::from_ref("List"),
        fields: vec![(Intern::from_ref("pointer"), Box::new(Ty::u8()))],
        resolved_params: Some(vec![(
            Intern::from_ref("x"),
            ast::ty::TyArg::Type(Box::new(Ty::u8())),
        )]),
    };
    assert_eq!(ty.sequence_element_type(), None);
}

#[test]
fn default_member_does_not_initialize_unassigned_bindings() {
    let mut parsed = TokenCursor::parse_source(
        "Default(value) has default value\nByte is in 0 to 255\nBox has Default\n    value Byte\n    Default.default: [value: 7]\nbox Box\n",
    );
    typecheck::prepare_parse_ast(&mut parsed, &flask::CompileTarget::Library);
    assert!(matches!(
        parsed.defs[&Intern::from_ref("box")].value,
        ast::BindValue::Unassigned
    ));
}

#[test]
fn entry_is_an_explicit_query_flag() {
    let parsed = TokenCursor::parse_source("main(): return ()\n#entry\nlaunch(): return ()\n");
    assert!(!parsed.defs[&Intern::from_ref("main")].attributes.entry);
    assert!(parsed.defs[&Intern::from_ref("launch")].attributes.entry);
}
