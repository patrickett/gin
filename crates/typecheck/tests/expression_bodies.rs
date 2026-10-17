mod support;

use ast::ConstValue;
use internment::Intern;
use support::check_with;
use typecheck::eval::{CompileTimeHost, Evaluator};
use typecheck::{DefId, TypedFileAst};

const PRELUDE: &str = "Int is in -128 to 127\nInt.finish(self): return\n";

fn evaluate(typed: &TypedFileAst, argument: i128, expected: i128) {
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
    let actual = evaluator
        .call(
            DefId::named(Intern::from_ref("choose")),
            &[ConstValue::Int(argument.into())],
        )
        .unwrap();
    assert_eq!(actual, ConstValue::Int(expected.into()));
}

#[test]
fn expression_bodies_return_values_with_or_without_annotations() {
    for source in [
        "choose(value Int): value\n",
        "choose(value Int) Int: value\n",
        "choose(value Int) Int := value\n",
        "choose(value Int): forward(value)\nforward(value Int): value\n",
        "choose(value Int) Int:\n    local(input Int): input\n    return local(value)\n",
    ] {
        let typed = check_with(PRELUDE, source);
        evaluate(&typed, 7, 7);
        let result = &typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type;
        assert!(matches!(result, ast::Ty::Named { name, .. } if name.as_str() == "Int"));
    }
}

#[test]
fn when_expression_bodies_infer_the_complete_integer_domain() {
    let typed = check_with(
        PRELUDE,
        "choose(value Int): when value < 0 then 256 else 0\n",
    );
    evaluate(&typed, -1, 256);
    evaluate(&typed, 1, 0);
    let result = &typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type;
    let domain = result.anonymous_integer_validity().unwrap().domain();
    assert!(domain.contains(256.into()));
    assert!(domain.contains(0.into()));
    assert!(!domain.contains(1.into()));
}

#[test]
fn multiline_when_expression_bodies_return_branch_values() {
    let typed = check_with(
        PRELUDE,
        r#"choose(value Int) Int: when value is
    0 then
        result: 7
        result
    else value
"#,
    );
    evaluate(&typed, 0, 7);
    evaluate(&typed, 4, 4);
}

#[test]
fn expression_bodies_infer_anonymous_result_alternatives() {
    let typed = check_with(
        PRELUDE,
        r#"pick(value Int): when value < 0 then Negative else Positive
choose(value Int) Int: when pick(value) is
    Negative then 7
    Positive then 3
"#,
    );
    evaluate(&typed, -1, 7);
    evaluate(&typed, 4, 3);
    let ast::Ty::ResultFamily { alternatives, .. } =
        &typed.defs[&DefId::named(Intern::from_ref("pick"))].return_type
    else {
        panic!("inferred result family");
    };
    assert_eq!(alternatives.len(), 2);
}

#[test]
fn when_expression_bodies_infer_a_union_of_named_types() {
    let typed = check_with(
        PRELUDE,
        r#"Left has value Int
Right has value Int
pick(value Int): when value < 0 then Left(value: 7) else Right(value: 3)
choose(value Int) Int: when pick(value) is
    Left(left) then left.value
    Right(right) then right.value
"#,
    );
    evaluate(&typed, -1, 7);
    evaluate(&typed, 4, 3);
    let ast::Ty::Union { variants, .. } =
        &typed.defs[&DefId::named(Intern::from_ref("pick"))].return_type
    else {
        panic!("inferred union");
    };
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].name.as_str(), "Left");
    assert_eq!(variants[1].name.as_str(), "Right");
}

#[test]
fn value_expression_bodies_require_exhaustive_branches() {
    let typed = check_with(PRELUDE, "choose(value Int) Int: when value < 0 then 7\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-missing-else-arm"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_when_expression_bodies_infer_union_values() {
    let typed = check_with(
        PRELUDE,
        r#"Left has value Int
Right has value Int
choose(value Int) Int:
    pick(input Int): when input < 0 then Left(value: 7) else Right(value: 3)
    return when pick(value) is
        Left(left) then left.value
        Right(right) then right.value
"#,
    );
    evaluate(&typed, -1, 7);
    evaluate(&typed, 4, 3);
}

#[test]
fn expression_bodies_check_declared_result_types() {
    let typed = check_with(
        PRELUDE,
        r#"Left has value Int
Right has value Int
choose(value Int) Left: when value < 0 then Left(value: 7) else Right(value: 3)
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-return-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn explicit_early_returns_can_exit_expression_bodies() {
    let typed = check_with(
        PRELUDE,
        "choose(value Int) Int: when value < 0 then\n    return 9\nelse value\n",
    );
    evaluate(&typed, -1, 9);
    evaluate(&typed, 4, 4);
}

#[test]
fn expression_bodies_preserve_reference_results() {
    let typed = check_with(PRELUDE, "choose(value ref Int) ref Int: value\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(matches!(
        typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type,
        ast::Ty::Ref { .. }
    ));
}

#[test]
fn expression_bodies_preserve_named_result_rows() {
    let typed = check_with(
        PRELUDE,
        "choose(value Int) first Int, second Int: value, 3\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let ast::Ty::Sequence(fields) =
        &typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type
    else {
        panic!("named result row");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, Some(Intern::from_ref("first")));
    assert_eq!(fields[1].name, Some(Intern::from_ref("second")));
}

#[test]
fn parameterless_expression_bodies_return_their_values() {
    let typed = check_with(
        PRELUDE,
        r#"constant: 7
zero: constant
choose(value Int):
    value.finish
    return zero
"#,
    );
    evaluate(&typed, 0, 7);
}

#[test]
fn matching_result_types_do_not_create_a_union() {
    let typed = check_with(
        PRELUDE,
        "choose(value Int): when value < 0 then value else value\n",
    );
    evaluate(&typed, -1, -1);
    evaluate(&typed, 4, 4);
    assert!(matches!(
        typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type,
        ast::Ty::Named { .. }
    ));
}

#[test]
fn nested_and_forwarded_when_unions_flatten_and_deduplicate_members() {
    for branch in [
        "when value < 0 then Left(value: 7) else Right(value: 3)",
        "pair(value)",
    ] {
        let typed = check_with(
            PRELUDE,
            &format!(
                r#"Left has value Int
Right has value Int
Third has value Int
pair(value Int): when value < 0 then Left(value: 7) else Right(value: 3)
pick(value Int): when value is
    0 then Third(value: 5)
    2 then Left(value: 9)
    else {branch}
choose(value Int) Int: when pick(value) is
    Left(left) then left.value
    Right(right) then right.value
    Third(third) then third.value
"#,
            ),
        );
        evaluate(&typed, -1, 7);
        evaluate(&typed, 0, 5);
        evaluate(&typed, 1, 3);
        evaluate(&typed, 2, 9);
        let ast::Ty::Union { variants, .. } =
            &typed.defs[&DefId::named(Intern::from_ref("pick"))].return_type
        else {
            panic!("flattened union");
        };
        assert_eq!(variants.len(), 3);
    }
}
