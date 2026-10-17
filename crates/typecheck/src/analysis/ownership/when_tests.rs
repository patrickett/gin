use super::*;
use crate::FileId;
use crate::test_support::push_expr;
use crate::typed::{TypedWhenArm, TypedWhenExpr};
use ast::{Literal, SpanId, SpanTable, Spanned, SubSpan};

fn literal(typed: &mut TypedFileAst, value: i128) -> ExprId {
    let id = push_expr(typed, TypedExprKind::Lit(Literal::Int(value.into())));
    typed.exprs.facts.const_facts.const_value[id.index()] =
        Some(ast::ConstValue::Int(value.into()));
    id
}

fn pattern(value: i128) -> Box<Spanned<ast::Pattern>> {
    Box::new(Spanned::new(
        ast::Pattern::Literal(Literal::Int(value.into()), SpanId::INVALID),
        SpanId::INVALID,
    ))
}

fn when_value(
    subject: Option<i128>,
    first: i128,
    conditional: Option<i128>,
    otherwise: i128,
) -> Option<ast::ConstValue> {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let subject = subject.map(|value| literal(&mut typed, value));
    let first = literal(&mut typed, first);
    let otherwise = literal(&mut typed, otherwise);
    let span = SubSpan::new(SpanId::INVALID);
    let mut arms = Vec::new();
    if subject.is_some() {
        arms.push(TypedWhenArm::Is {
            pattern: pattern(1),
            body: first,
            owned_bindings: vec![],
            arm_span: span,
        });
    }
    if let Some(value) = conditional {
        let body = literal(&mut typed, value);
        let flag = push_expr(
            &mut typed,
            TypedExprKind::FnCall {
                target: DefId::named(internment::Intern::from_ref("flag")),
                args: None,
                substituted_ty: None,
            },
        );
        arms.push(TypedWhenArm::Cond {
            coverage: Vec::new(),
            condition: crate::typed::TypedCondition::Is {
                subject: flag,
                pattern: pattern(1),
            },
            body,
            owned_bindings: vec![],
            arm_span: span,
        });
    }
    arms.push(TypedWhenArm::Else(otherwise, span));
    let expression = push_expr(
        &mut typed,
        TypedExprKind::When(TypedWhenExpr {
            subject,
            arms,
            place_joins: vec![],
            body_span: span,
        }),
    );
    closed_constant(&typed, expression)
}

#[test]
fn runtime_condition_prevents_selecting_the_later_else_arm() {
    assert_eq!(when_value(Some(0), 1, Some(2), 3), None);
}

#[test]
fn an_earlier_matching_pattern_still_selects_its_body() {
    assert_eq!(
        when_value(Some(1), 1, None, 3),
        Some(ast::ConstValue::Int(1.into()))
    );
}

#[test]
fn equal_condition_bodies_still_prove_a_constant() {
    assert_eq!(
        when_value(None, 9, Some(2), 2),
        Some(ast::ConstValue::Int(2.into()))
    );
}

#[test]
fn pattern_only_selection_still_uses_the_else_arm() {
    assert_eq!(
        when_value(Some(0), 1, None, 3),
        Some(ast::ConstValue::Int(3.into()))
    );
}

#[test]
fn mixed_forms_are_not_constants_even_when_the_first_pattern_matches() {
    assert_eq!(when_value(Some(1), 1, Some(2), 3), None);
}
