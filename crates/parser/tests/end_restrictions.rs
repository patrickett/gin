use ast::{
    Annotation, Binding, BindingConvention, Convention, Expr, Item, SourceFile, SyntaxNodeKind,
    TypePrefixKind,
};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("expected binding");
    };
    binding
}

#[test]
fn parameters_locals_and_neutral_headers_keep_written_end() {
    let source = concat!(
        "--- hé 🦀\n",
        "forward(tx end Transaction) Transaction: return tx\n",
        "root end Transaction: make()\n",
        "inferred end: make()\n",
        "header end\n",
        "work():\n",
        "    typed end Transaction: make()\n",
        "    local end: make()\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let function = binding(&file, 0);
    assert!(function.convention.is_none());
    let parameter = &function.parameters.as_ref().unwrap().value[0].value;
    let convention = parameter.convention.as_ref().unwrap();
    assert_eq!(convention.value.keyword.value, Convention::End);
    assert!(convention.value.group.is_none());
    for index in 1..=3 {
        let binding = binding(&file, index);
        assert!(binding.parameters.is_none());
        assert_eq!(
            binding.convention.as_ref().unwrap().value,
            BindingConvention::End
        );
        assert!(binding.result_binder.is_none());
        assert_eq!(binding.annotation.is_some(), index == 1);
    }
    for (offset, _) in source.match_indices("end") {
        for position in offset..offset + 3 {
            let hit = file.node_at_offset(position).unwrap();
            assert_eq!(hit.node.span.to_range(), offset..offset + 3);
            assert!(
                matches!(
                    hit.node.kind,
                    SyntaxNodeKind::Convention(keyword) if keyword.value == Convention::End
                ) || matches!(
                    hit.node.kind,
                    SyntaxNodeKind::BindingConvention(keyword) if keyword.value == BindingConvention::End
                )
            );
            assert!(!hit.declarations.is_empty());
        }
    }
    let end = source.find("header end").unwrap() + "header end".len();
    let hit = file.node_at_offset(end).unwrap();
    assert_eq!(hit.node.span.extract(source), "end");
    assert!(hit.ancestors.iter().any(|node| matches!(
        node.kind, SyntaxNodeKind::Binding(binding)
            if binding.name.value == "header" && node.span.extract(source) == "header end"
    )));
}

#[test]
fn absent_conventions_do_not_synthesize_owner_markers() {
    let file = parser::parse_strict(
        "forward(tx Transaction) Transaction: return tx\nlocal Transaction: make()\n",
    )
    .unwrap();
    assert!(binding(&file, 0).convention.is_none());
    assert!(binding(&file, 1).convention.is_none());
    assert!(
        binding(&file, 0).parameters.as_ref().unwrap().value[0]
            .value
            .convention
            .is_none()
    );
}

#[test]
fn explicit_results_and_fields_fail_at_end_and_recover() {
    for header in [
        "make() end Transaction",
        "make() result end Transaction",
        "Thing has make() end Transaction",
        "Thing has make() result end Transaction",
        "Thing has field end Transaction",
        "Thing has field end",
        "Thing has address end @Item",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err());
        assert_eq!(output.diagnostics[0].span.extract(&source), "end");
        assert_eq!(
            output.diagnostics[0].span.start(),
            source.find("end").unwrap()
        );
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later")
        );
    }
}

#[test]
fn end_restrictions_remain_separate_from_annotation_type_prefixes() {
    for (spelling, kind) in [
        ("own", TypePrefixKind::Own),
        ("ref", TypePrefixKind::Ref),
        ("mut", TypePrefixKind::Mut),
    ] {
        let source = format!(
            "forward(tx end {spelling} Item) Item: return tx\n\
             root end {spelling} Item: make()\n\
             work():\n    local end {spelling} Item: make()\n"
        );
        let file = parser::parse_strict(&source).unwrap();
        let parameter = &binding(&file, 0).parameters.as_ref().unwrap().value[0].value;
        assert_eq!(
            parameter.convention.as_ref().unwrap().value.keyword.value,
            Convention::End
        );
        for annotation in [
            parameter.annotation.as_ref().unwrap(),
            binding(&file, 1).annotation.as_ref().unwrap(),
        ] {
            let Annotation::Expression(expression) = &annotation.value else {
                panic!("type annotation");
            };
            let Expr::PrefixedType(prefix) = &expression.value else {
                panic!("prefixed type");
            };
            assert_eq!(prefix.keyword.value, kind);
        }
        for (offset, _) in source.match_indices(&format!("{spelling} Item")) {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(
                hit.node.kind, SyntaxNodeKind::TypePrefixKeyword(keyword) if keyword.value == kind
            ));
            assert_eq!(hit.node.span.extract(&source), spelling);
        }
    }
}

#[test]
fn malformed_restrictions_never_become_successful_annotations() {
    for header in [
        "tx end end Transaction",
        "tx own end Transaction",
        "tx end{group} Transaction",
        "tx end{ Transaction",
        "f(tx end end Transaction)",
        "f(tx own end Transaction)",
        "f(tx ref end Transaction)",
        "f(tx mut end Transaction)",
        "f(tx ref{group} end Transaction)",
        "f(tx end{group} Transaction)",
        "f(tx end{} Transaction)",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        assert!(
            output
                .file
                .declaration_outline()
                .any(|entry| entry.name == "later"),
            "{source}"
        );
    }
}

#[test]
fn multiline_end_parameters_remain_inside_the_parameter_list() {
    let source = "read(\n    tx end Transaction\n) Transaction: return tx\n";
    let file = parser::parse_strict(source).unwrap();
    let parameters = &binding(&file, 0).parameters.as_ref().unwrap().value;
    assert_eq!(parameters.len(), 1);
    assert_eq!(
        parameters[0]
            .value
            .convention
            .as_ref()
            .unwrap()
            .value
            .keyword
            .value,
        Convention::End
    );
}

#[test]
fn ordinary_end_identifiers_remain_names() {
    let source = concat!(
        "end := 1\n",
        "read(end Transaction) Transaction: return end\n",
        "value := object.end(end)\n",
        "named := f(\nend: 1\n)\n",
        "Thing has end Transaction\n",
    );
    let file = parser::parse_strict(source).unwrap();
    assert_eq!(binding(&file, 0).name.value, "end");
    assert!(
        binding(&file, 1).parameters.as_ref().unwrap().value[0]
            .value
            .convention
            .is_none()
    );
    for (offset, _) in source.match_indices("end") {
        assert!(matches!(
            file.node_at_offset(offset).unwrap().node.kind,
            SyntaxNodeKind::Name("end")
        ));
    }
}
