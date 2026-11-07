//! Source representation only: none of these assertions establish checked ownership or privacy.
use ast::{
    Annotation, Argument, DeclarationKind, DomainBase, Expr, FieldAttribute, Item,
    RefinementPredicate, Statement, SyntaxNodeKind, TypeBody, TypeMember,
};

const WINDOW: &str = concat!(
    "--- hé 🦀\n",
    "Window(element) has\n",
    "    #private\n",
    "    payload element\n",
    "    #private\n",
    "    lower in 0 to 16\n",
    "    #private\n",
    "    upper in 0 to 16 and >= lower\n",
    "\n",
    "make_window(element Type, payload element,\n",
    "            lower in 0 to 16, upper in 0 to 16 and >= lower) Window(element):\n",
    "    return Window(payload: payload, lower: lower, upper: upper)\n",
    "\n",
    "rebuild_window(element Type, window Window(element)) Window(element):\n",
    "    Window(payload, lower, upper) := window\n",
    "    return Window(payload: payload, lower: lower, upper: upper)\n",
    "\n",
    "take_window(element Type, window Window(element)) element:\n",
    "    Window(payload, lower, upper) := window\n",
    "    return payload\n",
);

#[test]
fn actual_window_and_unrelated_names_retain_raw_source_roles() {
    for source in [
        WINDOW.to_owned(),
        WINDOW
            .replace("Window", "Parcel")
            .replace("window", "parcel")
            .replace("payload", "cargo")
            .replace("lower", "start")
            .replace("upper", "finish")
            .replace("element", "content"),
    ] {
        let file = parser::parse_strict(&source).unwrap();
        assert_eq!(file.declaration_outline().count(), 4);
        let Item::Type(declaration) = &file.items[0].value else {
            panic!("aggregate");
        };
        assert_eq!(declaration.parameters.as_ref().unwrap().value.len(), 1);
        let TypeBody::Has(members) = &declaration.body.value else {
            panic!("fields");
        };
        assert_eq!(members.len(), 3);
        for member in members {
            let TypeMember::Binding(field) = &member.value else {
                panic!("ordinary field");
            };
            assert_eq!(field.attributes.len(), 1);
            let attribute = &field.attributes[0];
            let FieldAttribute::Private {
                hash,
                private_keyword,
            } = attribute.value
            else {
                panic!("private attribute");
            };
            for (span, spelling) in [(hash, "#"), (private_keyword, "private")] {
                let span = file.span_table.get(span);
                assert_eq!(span.extract(&source), spelling);
                let hit = file.node_at_offset(span.start()).unwrap();
                assert_eq!(hit.node.span, span);
                assert!(matches!(
                    hit.node.kind,
                    SyntaxNodeKind::AttributeHash | SyntaxNodeKind::PrivateKeyword
                ));
                assert_eq!(hit.declarations.len(), 2);
            }
        }
        let TypeMember::Binding(upper) = &members[2].value else {
            panic!("upper field");
        };
        let Annotation::Domain { domain, .. } = &upper.annotation.as_ref().unwrap().value else {
            panic!("literal-bounded metadata");
        };
        assert!(matches!(domain.value.base.value, DomainBase::Range { .. }));
        assert_eq!(domain.value.refinements.len(), 1);
        let RefinementPredicate::ImplicitComparison { right, .. } =
            &domain.value.refinements[0].value.predicate.value
        else {
            panic!("cross-field refinement");
        };
        let TypeMember::Binding(lower) = &members[1].value else {
            panic!("lower field");
        };
        assert_eq!(right.value, Expr::Name(lower.name.value.clone()));
        for item in &file.items[2..] {
            let Item::Binding(function) = &item.value else {
                panic!("function");
            };
            let Expr::Block(body) = &function.value.as_ref().unwrap().expression.value else {
                panic!("body");
            };
            let Statement::Destructure(pattern) = &body.statements[0].value else {
                panic!("explicit pattern, not a call");
            };
            let fields = pattern.pattern.value.fields.as_ref().unwrap();
            assert_eq!(fields.binders.value.len(), 3);
            for binder in &fields.binders.value {
                let span = file.span_table.get(binder.span_id);
                let hit = file.node_at_offset(span.start()).unwrap();
                assert!(matches!(hit.node.kind, SyntaxNodeKind::PatternBinder(_)));
                assert_eq!(hit.node.span, span);
                assert_eq!(
                    hit.declarations.last().unwrap().kind,
                    DeclarationKind::PatternBinder
                );
                assert_eq!(hit.declarations.last().unwrap().name, binder.value);
                assert_eq!(hit.declarations[0].name, function.name.value);
            }
            for delimiter in [fields.open, fields.close] {
                let span = file.span_table.get(delimiter);
                let hit = file.node_at_offset(span.start()).unwrap();
                assert!(matches!(hit.node.kind, SyntaxNodeKind::PatternDelimiter(_)));
                assert_eq!(hit.node.span, span);
            }
            let operator_span = file.span_table.get(pattern.operator.span_id);
            assert_eq!(operator_span.extract(&source), ":=");
            let hit = file.node_at_offset(operator_span.start()).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::BindingOperator(_)));
            let rhs_span = file.span_table.get(pattern.value.span_id);
            let hit = file.node_at_offset(rhs_span.start()).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(_)));
            assert_eq!(hit.declarations.len(), 1);
        }
        let Item::Binding(make) = &file.items[1].value else {
            panic!("constructor body");
        };
        let Expr::Block(body) = &make.value.as_ref().unwrap().expression.value else {
            panic!("body");
        };
        let Statement::Return(returned) = &body.statements[0].value else {
            panic!("return");
        };
        let Expr::Call { arguments, .. } = &returned.value.as_ref().unwrap().value else {
            panic!("construction remains a call");
        };
        assert!(
            arguments
                .value
                .iter()
                .all(|argument| matches!(argument.value, Argument::Named { .. }))
        );
    }
}

#[test]
fn every_utf8_edit_prefix_and_modified_delimiter_remains_queryable() {
    for end in (0..=WINDOW.len()).filter(|end| WINDOW.is_char_boundary(*end)) {
        let source = &WINDOW[..end];
        let output = parser::parse(source).unwrap();
        for offset in (0..=source.len()).filter(|offset| source.is_char_boundary(*offset)) {
            assert!(output.file.node_at_offset(offset).is_some());
        }
    }
    for source in [
        WINDOW.replacen("Window(element)", "Window(element", 1),
        WINDOW.replacen(
            "Window(payload, lower, upper)",
            "Window(payload, lower, upper",
            1,
        ),
        WINDOW.replacen("#private", "#private(", 1),
    ] {
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert!(parser::parse_strict(&source).is_err());
        for offset in (0..=source.len()).filter(|offset| source.is_char_boundary(*offset)) {
            assert!(output.file.node_at_offset(offset).is_some());
        }
        assert!(
            output
                .file
                .declaration_outline()
                .any(|item| item.name == "take_window")
        );
    }
}

#[test]
fn sequential_patterns_in_nested_bodies_keep_local_ancestry_and_source_order() {
    let source = concat!(
        "outer():\n",
        "    inner(value Parcel):\n",
        "        Parcel(child, limit) := value\n",
        "        Envelope(payload) := child\n",
        "        return payload\n",
        "    Marker() := marker\n",
        "    return\n",
        "later := 1\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(
        file.declaration_outline()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["outer", "later"]
    );
    let hit = file
        .node_at_offset(source.find("Envelope").unwrap())
        .unwrap();
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["outer", "inner"]
    );
    let offset = source.find("Envelope(payload)").unwrap() + "Envelope(".len();
    let hit = file.node_at_offset(offset).unwrap();
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["outer", "inner", "payload"]
    );
    let hit = file.node_at_offset(source.find("Marker").unwrap()).unwrap();
    assert_eq!(
        hit.declarations
            .iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
        ["outer"]
    );
}
