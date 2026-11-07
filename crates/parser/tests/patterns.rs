use ast::{
    BindingOperator, Block, DestructureStatement, Expr, Item, SourceFile, SpanId, Statement,
};

fn body(file: &SourceFile) -> &Block {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("outer binding");
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("binding body");
    };
    block
}

fn pattern(file: &SourceFile, index: usize) -> &DestructureStatement {
    let Statement::Destructure(pattern) = &body(file).statements[index].value else {
        panic!("flat consuming pattern");
    };
    pattern
}

fn text(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn flat_patterns_preserve_constructor_binders_punctuation_and_rhs() {
    let source = "--- hé 🦀\nmain:\n    Pair(left, right) := make(value)\n    return left\n";
    let file = parser::parse_strict(source).unwrap();
    let pattern = pattern(&file, 0);
    let fields = pattern.pattern.value.fields.as_ref().unwrap();
    assert_eq!(pattern.pattern.value.constructor.value, "Pair");
    assert_eq!(
        text(&file, pattern.pattern.value.constructor.span_id),
        "Pair"
    );
    assert_eq!(text(&file, fields.binders.span_id), "(left, right)");
    assert_eq!(text(&file, fields.open), "(");
    assert_eq!(text(&file, fields.close), ")");
    let names: Vec<_> = fields
        .binders
        .value
        .iter()
        .map(|name| name.value.as_str())
        .collect();
    assert_eq!(names, ["left", "right"]);
    for name in &fields.binders.value {
        assert_eq!(text(&file, name.span_id), name.value);
    }
    assert_eq!(pattern.operator.value, BindingOperator::ColonEquals);
    assert_eq!(text(&file, pattern.operator.span_id), ":=");
    assert!(matches!(pattern.value.value, Expr::Call { .. }));
    assert_eq!(text(&file, pattern.value.span_id), "make(value)");
    assert_eq!(
        text(&file, body(&file).statements[0].span_id),
        "Pair(left, right) := make(value)"
    );
    assert!(matches!(
        body(&file).statements[1].value,
        Statement::Return(_)
    ));
}

#[test]
fn duplicate_empty_and_multiline_binders_are_syntax_not_semantic_checks() {
    let source = concat!(
        "main:\n",
        "    Pair(value, value,) := pair\n",
        "    Empty() := empty\n",
        "    Triple(\n        first,\n        second\n    ) := triple\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let duplicate = pattern(&file, 0).pattern.value.fields.as_ref().unwrap();
    assert_eq!(duplicate.binders.value.len(), 2);
    assert_eq!(
        duplicate.binders.value[0].value,
        duplicate.binders.value[1].value
    );
    assert_ne!(
        duplicate.binders.value[0].span_id,
        duplicate.binders.value[1].span_id
    );
    let empty = pattern(&file, 1).pattern.value.fields.as_ref().unwrap();
    assert!(empty.binders.value.is_empty());
    assert_eq!(text(&file, empty.binders.span_id), "()");
    assert_eq!(
        pattern(&file, 2)
            .pattern
            .value
            .fields
            .as_ref()
            .unwrap()
            .binders
            .value
            .len(),
        2
    );
}

#[test]
fn body_only_disambiguation_preserves_calls_and_other_binding_headers() {
    let source = concat!(
        "pair(value) := value\n",
        "Thing has\n    method(value) := value\n",
        "main:\n",
        "    Pair(value)\n",
        "    local(value): value\n",
        "    typed(value) Int := value\n",
        "    inferred(value) := value\n",
        "    constant() := 2\n",
        "    Pair(value) := pair\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(top) = &file.items[0].value else {
        panic!("function")
    };
    assert!(top.parameters.is_some());
    let Item::Binding(main) = &file.items[2].value else {
        panic!("main")
    };
    let Expr::Block(block) = &main.value.as_ref().unwrap().expression.value else {
        panic!("body");
    };
    assert!(matches!(
        block.statements[0].value,
        Statement::Expression(_)
    ));
    assert!(matches!(block.statements[1].value, Statement::Binding(_)));
    assert!(matches!(block.statements[2].value, Statement::Binding(_)));
    assert!(matches!(block.statements[3].value, Statement::Binding(_)));
    assert!(matches!(block.statements[4].value, Statement::Binding(_)));
    assert!(matches!(
        block.statements[5].value,
        Statement::Destructure(_)
    ));
}

#[test]
fn unsupported_patterns_diagnose_and_recover_before_following_statements() {
    for rejected in [
        "Pair(Inner(value), other) := pair",
        "Pair(name: 1) := pair",
        "Pair(value Int) := pair",
        "Pair(value ref) := pair",
        "Pair(,value) := pair",
        "Pair(value,, other) := pair",
        "Pair(value) :: pair",
        "Pair(value) += pair",
        "module.Pair(value) := pair",
    ] {
        let source = format!("main:\n    {rejected}\n    return value\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{rejected}");
        assert!(parser::parse_strict(&source).is_err(), "{rejected}");
        assert!(matches!(
            body(&output.file).statements[0].value,
            Statement::Error
        ));
        assert!(matches!(
            body(&output.file).statements[1].value,
            Statement::Return(_)
        ));
        let Item::Binding(later) = &output.file.items[1].value else {
            panic!("later")
        };
        assert_eq!(later.name.value, "later");
    }
}

#[test]
fn pattern_and_modified_headers_recover_after_an_unfinished_expression_call() {
    for header in ["next end Parcel", "next ref Parcel", "next mut Parcel"] {
        let source = format!(
            "main:\n    pending := consume(\n    Parcel(value) := parcel\n    {header} := parcel\n    return value\nlater := 2\n"
        );
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{header}");
        let block = body(&output.file);
        assert_eq!(block.statements.len(), 4, "{header}: {block:#?}");
        assert!(matches!(
            block.statements[1].value,
            Statement::Destructure(_)
        ));
        assert!(matches!(block.statements[2].value, Statement::Binding(_)));
        assert!(matches!(block.statements[3].value, Statement::Return(_)));
        assert_eq!(output.file.items.len(), 2);
    }
}

#[test]
fn missing_value_is_explicit_recovery_not_a_valid_pattern() {
    let output = parser::parse("main:\n    Pair(value) :=\n    return value\n").unwrap();
    assert!(output.has_errors());
    assert_eq!(pattern(&output.file, 0).value.value, Expr::Missing);
    assert_eq!(
        text(&output.file, pattern(&output.file, 0).value.span_id),
        ""
    );
    assert!(matches!(
        body(&output.file).statements[1].value,
        Statement::Return(_)
    ));
}

#[test]
fn pattern_values_obey_expression_depth_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};
    let failure = parser::parse_with_options(
        "main:\n    Pair(value) := - - -value\n",
        ParseOptions {
            limits: ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        failure,
        ParseFailure::LimitExceeded {
            limit: ParseLimit::ExpressionDepth,
            ..
        }
    ));
}

/// `_` is a binder that discards its field; it is an ordinary name token to the parser and
/// the checker gives it meaning, so any number may appear in one pattern.
#[test]
fn a_wildcard_is_a_binder_that_discards() {
    let source = "main:\n    Pair(_, _) := pair\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Destructure(pattern) = &body(&file).statements[0].value else {
        panic!("destructure")
    };
    let binders = &pattern.pattern.value.fields.as_ref().unwrap().binders.value;
    assert_eq!(binders.len(), 2);
    assert!(binders.iter().all(|binder| binder.value == "_"));
}

#[test]
fn a_binder_may_name_its_field() {
    let source = "main:\n    Pair(second: right, left) := pair\n    return second\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Destructure(destructure) = &body(&file).statements[0].value else {
        panic!("a destructure");
    };
    let fields = destructure.pattern.value.fields.as_ref().unwrap();
    assert_eq!(fields.binders.value.len(), 2);
    assert_eq!(
        fields.selectors.iter().map(Option::is_some).collect::<Vec<_>>(),
        [true, false]
    );
}
