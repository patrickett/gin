use super::*;
use crate::FileId;
use crate::test_support::push_expr;
use ast::{HashFloat, SpanTable, Ty};

fn expression() -> (TypedFileAst, ExprId) {
    let mut typed = TypedFileAst::new(FileId(0), SpanTable::new());
    let id = push_expr(&mut typed, TypedExprKind::Invalid);
    (typed, id)
}

#[test]
fn integer_replacement_preserves_the_selected_type() {
    let (mut typed, id) = expression();
    let ty = Ty::bounded_int((-128).into(), 127.into());
    typed.exprs.ty[id.index()] = ty.clone();
    let value = ConstValue::Int((-7).into());
    replace_with_scalar_value(&mut typed, id, value.clone());
    assert_eq!(
        typed.exprs.kind[id.index()],
        TypedExprKind::Lit(Literal::Int((-7).into()))
    );
    assert_eq!(typed.exprs.ty[id.index()], ty);
    assert_eq!(typed.exprs.const_value_of(id), Some(&Some(value)));
}

#[test]
fn float_replacement_preserves_literal_and_type_value() {
    let (mut typed, id) = expression();
    let value = HashFloat(1.5);
    replace_with_scalar_value(&mut typed, id, ConstValue::Float(value));
    assert_eq!(
        typed.exprs.kind[id.index()],
        TypedExprKind::Lit(Literal::Float(value))
    );
    assert_eq!(typed.exprs.ty[id.index()], Ty::Float { value: Some(value) });
    assert_eq!(
        typed.exprs.const_value_of(id),
        Some(&Some(ConstValue::Float(value)))
    );
}

#[test]
fn text_replacement_uses_utf8_byte_length_including_empty_text() {
    for text in ["", "abc", "é!"] {
        let (mut typed, id) = expression();
        let value = ConstValue::Text(text.to_string());
        replace_with_scalar_value(&mut typed, id, value.clone());
        assert_eq!(
            typed.exprs.kind[id.index()],
            TypedExprKind::Lit(Literal::String(text.to_string()))
        );
        assert_eq!(
            typed.exprs.ty[id.index()],
            Ty::Array {
                elem: Box::new(Ty::u8()),
                size: ast::NormalExpr::from(text.len() as i128)
            }
        );
        assert_eq!(typed.exprs.const_value_of(id), Some(&Some(value)));
    }
}

#[test]
fn unsupported_replacement_fails_before_mutating_the_expression() {
    let unsupported = [
        ConstValue::Domain(std::sync::Arc::new(ast::integer::IntegerDomain::unbounded())),
        ConstValue::Tag {
            name: Intern::from_ref("True"),
            qual_path: None,
            args: Default::default(),
        },
        ConstValue::ResultAlternative {
            owner: ast::ResultFamilyOwner::LocalCallable(ast::BinderId::new(
                0,
                ast::BinderOwner::Expression(0),
            )),
            label: Intern::from_ref("True"),
        },
        ConstValue::Record {
            fields: Default::default(),
        },
        ConstValue::ResultRow(Default::default()),
        ConstValue::FixedArray(Default::default()),
        ConstValue::Sequence(Default::default()),
        ConstValue::SequenceRepeat {
            seed: Box::new(ConstValue::ZERO),
            count: Box::new(ast::NormalExpr::from(2)),
        },
    ];
    for value in unsupported {
        let (mut typed, id) = expression();
        let before = typed.exprs.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            replace_with_scalar_value(&mut typed, id, value.clone())
        }));
        assert!(
            result.is_err(),
            "unsupported replacement succeeded: {value:?}"
        );
        assert_eq!(typed.exprs, before);
    }
}
