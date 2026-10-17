use super::*;

fn repeated_literals(order: &[u64]) -> ast::Pattern {
    ast::Pattern::Generic {
        name: Intern::from_ref("Many"),
        params: vec![
            (
                Intern::from_ref("0"),
                ParameterKind::Tagged(Box::new(Spanned::new(
                    ast::Expr::Lit(ast::Literal::Int(0.into())),
                    SpanId::INVALID,
                ))),
            ),
            (
                Intern::from_ref("1"),
                ParameterKind::Tagged(Box::new(Spanned::new(
                    ast::Expr::Lit(ast::Literal::Int(1.into())),
                    SpanId::INVALID,
                ))),
            ),
        ],
        param_spans: order
            .iter()
            .map(|n| (Intern::new(n.to_string()), SpanId::INVALID))
            .collect(),
        span: SpanId::INVALID,
    }
}

#[test]
fn constructor_keys_preserve_repeated_argument_positions() {
    assert_ne!(
        pattern_surface(&repeated_literals(&[0, 0, 1])),
        pattern_surface(&repeated_literals(&[0, 1, 0]))
    );
}

#[test]
fn constructor_constants_preserve_repeated_argument_positions() {
    let mut typed = TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    typed
        .variant_map
        .insert(Intern::from_ref("Many"), Vec::new());
    let expected = ast::ConstValue::Tag {
        name: Intern::from_ref("Many"),
        qual_path: None,
        args: vec![
            ast::ConstValue::Int(0.into()),
            ast::ConstValue::Int(0.into()),
            ast::ConstValue::Int(1.into()),
        ]
        .into(),
    };
    assert_eq!(
        pattern_value_const(&repeated_literals(&[0, 0, 1]), &typed),
        Some(expected)
    );
}

#[test]
fn missing_constructor_argument_kind_cannot_produce_a_constant() {
    let mut typed = TypedFileAst::new(crate::FileId(0), ast::SpanTable::new());
    typed
        .variant_map
        .insert(Intern::from_ref("Many"), Vec::new());
    let mut pattern = repeated_literals(&[0, 0, 1]);
    if let ast::Pattern::Generic { params, .. } = &mut pattern {
        params.pop();
    }
    assert_eq!(pattern_value_const(&pattern, &typed), None);
}
