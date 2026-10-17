use ast::{BindValue, Expr, WhenArm};
use diagnostic::Category;
use parser::query::SourceParseExt;

fn parse(source: &str) -> parser::query::ParseOutput {
    let parsed = source.parse_source_full();
    assert!(
        parsed.symptoms.iter().all(|d| d.category != Category::Flaw),
        "{source}\n{:?}",
        parsed.symptoms
    );
    parsed
}

#[test]
fn case_bodies_are_value_blocks_with_explicit_returns_preserved() {
    let parsed = parse(
        "choose(captured Int) Int:\n    value: when\n        captured < 0 then\n            adjusted: -captured\n            adjusted\n        else\n            captured\n    return value\n",
    );
    let BindValue::Body { exprs, .. } =
        &parsed.ast.defs[&internment::Intern::from_ref("choose")].value
    else {
        panic!()
    };
    let Expr::Bind(bind) = &exprs[0].value else {
        panic!()
    };
    let BindValue::Expr(value) = &bind.value else {
        panic!()
    };
    let Expr::When(branch) = &value.value else {
        panic!()
    };
    let WhenArm::Cond { body, .. } = &branch.arms[0] else {
        panic!()
    };
    assert!(matches!(&body.value, Expr::Block(items) if items.len() == 2));
    parse(
        "choose(captured Int) Int:\n    when captured < 0 then\n        return 0\n    return captured\n",
    );
}

#[test]
fn shared_subject_patterns_comparisons_and_continued_guards() {
    parse(
        "value: when captured\n    < 0 then 'negative'\n    = 0 then 'zero'\n    else 'positive'\n",
    );
    parse(
        "value: when result is\n    Some(value) and value > 0 then value\n    Some(value) then 0\n    None then -1\n",
    );
    parse(
        "value: when\n    name.starts_with('_')\n        and name.tail() is Some(suffix)\n        and suffix.parse_int() is Some(index)\n        and index >= 0\n        and index < arity\n    then Valid(index, name)\n    else Invalid(name)\n",
    );
}

#[test]
fn aligned_compact_branches_keep_the_following_statement_outside_the_when() {
    let parsed = parse(
        "choose(value Int) Int:\n    selected: when value < 0 then -value\n              else value\n    return selected\n",
    );
    let BindValue::Body { exprs, ret } =
        &parsed.ast.defs[&internment::Intern::from_ref("choose")].value
    else {
        panic!()
    };
    assert_eq!(exprs.len(), 1);
    assert_eq!(ret.values.len(), 1);
    let Expr::Bind(bind) = &exprs[0].value else {
        panic!()
    };
    let BindValue::Expr(value) = &bind.value else {
        panic!()
    };
    let Expr::When(branch) = &value.value else {
        panic!()
    };
    assert_eq!(branch.arms.len(), 2);
}

#[test]
fn old_if_and_case_markers_are_rejected() {
    for source in [
        "value: if ready\n    return 1\n",
        "value: when\n    case ready then 1\n    else 0\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.iter().any(|d| d.category == Category::Flaw));
    }
}

#[test]
fn nested_is_blocks_preserve_leaf_order_bodies_and_outer_fallbacks() {
    let parsed = parse(
        "value: when input is\n    Some(x) and x is\n        Left(y) and y > 0 then\n            adjusted: y + 1\n            adjusted\n        Right(_) then 2\n    Some(_) then 3\n    None then 4\nnext: 5\n",
    );
    let BindValue::Expr(value) = &parsed.ast.defs[&internment::Intern::from_ref("value")].value
    else {
        panic!()
    };
    let Expr::When(branch) = &value.value else {
        panic!()
    };
    assert!(branch.subject.is_some());
    assert_eq!(branch.arms.len(), 4);
    let WhenArm::Cond {
        condition, body, ..
    } = &branch.arms[0]
    else {
        panic!()
    };
    assert!(matches!(condition, ast::Condition::And(..)));
    assert!(matches!(&body.value, Expr::Block(items) if items.len() == 2));
    assert!(
        parsed
            .ast
            .defs
            .contains_key(&internment::Intern::from_ref("next"))
    );
}

#[test]
fn nested_blocks_can_have_local_else_and_multiple_levels() {
    parse(
        "value: when input is\n    Some(x) and x is\n        Left(y) and y is\n            0 then 1\n            else 2\n        Right(_) then 3\n        else 4\n    None then 5\n",
    );
    parse("value: when ready and input is\n    Some(x) then x\n    None then 0\n");
    parse(
        "value: when\n    ready and input is\n        Some(x) then x\n        None then 0\n    else -1\n",
    );
}

#[test]
fn malformed_nested_blocks_are_rejected() {
    for source in [
        "value: when input is\n    Some(x) and x is\n    None then 0\n",
        "value: when input is\n    Some(x) and x is\n        else 1\n        Left(_) then 2\n    None then 0\n",
        "value: when input is\n    Some(x) and x is\n        Left(_)\n    None then 0\n",
    ] {
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.iter().any(|d| d.category == Category::Flaw),
            "{source}"
        );
    }
}

#[test]
fn nested_constructor_pattern_structure() {
    let parsed = parse("value: when input is\n    Some(Left(y)) and y > 0 then y\n    else 0\n");
    let BindValue::Expr(value) = &parsed.ast.defs[&internment::Intern::from_ref("value")].value
    else {
        panic!()
    };
    let Expr::When(branch) = &value.value else {
        panic!()
    };
    let WhenArm::Cond { condition, .. } = &branch.arms[0] else {
        panic!()
    };
    let ast::Condition::And(left, _) = condition else {
        panic!()
    };
    assert!(matches!(left.as_ref(), ast::Condition::And(..)));
}

#[test]
fn comparison_alternatives_do_not_extend_inline_bodies() {
    parse("value: when input\n    < 4 then 1 + 1\n    < 8 then 3\n    else 4\n");
    parse(
        "choose(input Int) Int:\n    when input\n        < 4 then return 1\n        < 8 then return 2 + 1\n        else return 4\n",
    );
    parse("case: 1\n");
}
