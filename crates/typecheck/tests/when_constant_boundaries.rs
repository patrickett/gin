use ast::{BindValue, ConstValue, Expr};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::analysis::{CompTimeEvaluator, ConstEnv};

fn source(subject: &str) -> ast::FileAst {
    let parsed =
        format!("choice: when {subject} is\n    1 then 1\n    else 3\n").parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    parsed.ast
}

fn expression(file: &ast::FileAst) -> &ast::Typed<Expr> {
    let BindValue::Expr(expression) = &file.defs[&Intern::from_ref("choice")].value else {
        panic!("expected expression body");
    };
    expression
}

fn expression_mut(file: &mut ast::FileAst) -> &mut ast::Typed<Expr> {
    let BindValue::Expr(expression) = &mut file
        .defs
        .get_mut(&Intern::from_ref("choice"))
        .unwrap()
        .value
    else {
        panic!("expected expression body");
    };
    expression
}

#[test]
fn intrinsic_folding_preserves_patterns_without_a_subject() {
    for subject in ["0", "1"] {
        let mut file = source(subject);
        let Expr::When(when) = &mut expression_mut(&mut file).value else {
            panic!()
        };
        when.subject = None;
        typecheck::inject_compiler_intrinsics(&mut file);
        assert!(matches!(expression(&file).value, Expr::When(_)));
    }
}

#[test]
fn intrinsic_folding_still_selects_an_earlier_matching_pattern() {
    let mut file = source("1");
    let Expr::When(when) = &mut expression_mut(&mut file).value else {
        panic!()
    };
    when.subject.as_mut().unwrap().const_value = Some(Box::new(ConstValue::ONE));
    typecheck::inject_compiler_intrinsics(&mut file);
    assert!(matches!(expression(&file).value, Expr::Lit(_)));
    assert_eq!(
        CompTimeEvaluator::new(&ConstEnv::default(), &file).eval(&expression(&file).value),
        Some(ConstValue::ONE)
    );
}

#[test]
fn an_unknown_pattern_subject_is_not_replaced_with_zero() {
    let file = source("unknown");
    assert_eq!(
        CompTimeEvaluator::new(&ConstEnv::default(), &file).eval(&expression(&file).value),
        None
    );
}

#[test]
fn subjectless_conditions_still_evaluate_at_compile_time() {
    let parsed = "choice: when 1 = 1 then 7 else 9\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_eq!(
        CompTimeEvaluator::new(&ConstEnv::default(), &parsed.ast)
            .eval(&expression(&parsed.ast).value),
        Some(ConstValue::Int(7.into()))
    );
}

#[test]
fn guarded_patterns_select_the_first_matching_case() {
    let parsed = "choice: when 2 is\n    value and value < 0 then 7\n    value and value > 0 then value\n    else 9\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    assert_eq!(
        CompTimeEvaluator::new(&ConstEnv::default(), &parsed.ast)
            .eval(&expression(&parsed.ast).value),
        Some(ConstValue::Int(2.into()))
    );
}
