use ast::{
    Annotation, Binding, Convention, Expr, Item, SourceFile, Statement, SyntaxNodeKind, TypeBody,
    TypeMember, TypePrefixKind,
};

fn binding(file: &SourceFile, index: usize) -> &Binding {
    let Item::Binding(binding) = &file.items[index].value else {
        panic!("ordinary binding");
    };
    binding
}

fn members(file: &SourceFile, index: usize) -> Vec<&Binding> {
    let Item::Type(declaration) = &file.items[index].value else {
        panic!("type declaration");
    };
    let TypeBody::Has(members) = &declaration.body.value else {
        panic!("has members");
    };
    members
        .iter()
        .map(|member| match &member.value {
            TypeMember::Binding(binding) => binding.as_ref(),
            TypeMember::Provision(_) | TypeMember::ProvisionApplied(_) | TypeMember::Error => {
                panic!("supported member")
            }
        })
        .collect()
}

fn own_type(binding: &Binding) -> &ast::PrefixedType {
    let Annotation::Expression(expression) = &binding.annotation.as_ref().unwrap().value else {
        panic!("written type annotation");
    };
    let Expr::PrefixedType(prefix) = &expression.value else {
        panic!("written type prefix");
    };
    assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
    prefix
}

#[test]
fn every_written_convention_is_a_suffix_not_an_inferred_receiver_mode() {
    for (word, expected) in [
        ("ref", Convention::Ref),
        ("mut", Convention::Mut),
        ("own", Convention::Own),
    ] {
        let source = format!(
            "Thing has method(self {word}, other {word} Byte): return other\n\
             ordinary(value {word}): return value\n\
             Generic(element {word} Type) is Ready\n"
        );
        let file = parser::parse_strict(&source).unwrap();
        let method_members = members(&file, 0);
        let method = method_members[0];
        let parameters = &method.parameters.as_ref().unwrap().value;
        assert_eq!(parameters.len(), 2);
        assert_eq!(parameters[0].value.name.value, "self");
        assert!(parameters[0].value.annotation.is_none());
        assert!(parameters[0].value.default.is_none());
        for parameter in parameters {
            let convention = parameter.value.convention.as_ref().unwrap();
            assert_eq!(convention.value.keyword.value, expected);
            assert_eq!(
                file.span_table.get(convention.span_id).extract(&source),
                word
            );
        }
        let ordinary = &binding(&file, 1).parameters.as_ref().unwrap().value[0].value;
        assert_eq!(
            ordinary.convention.as_ref().unwrap().value.keyword.value,
            expected
        );
        assert!(ordinary.annotation.is_none());
        let Item::Type(generic) = &file.items[2].value else {
            panic!("generic declaration");
        };
        let parameter = &generic.parameters.as_ref().unwrap().value[0].value;
        assert_eq!(
            parameter.convention.as_ref().unwrap().value.keyword.value,
            expected
        );
        assert!(parameter.annotation.is_some());
    }
    let file =
        parser::parse_strict("Thing has method(self, other Byte): return self\n").unwrap();
    let method_members = members(&file, 0);
    let parameters = &method_members[0].parameters.as_ref().unwrap().value;
    assert!(
        parameters
            .iter()
            .all(|parameter| parameter.value.convention.is_none())
    );
    assert!(parameters[0].value.annotation.is_none());
}

#[test]
fn type_prefix_and_parameter_keywords_have_distinct_query_roles_and_owners() {
    let source = concat!(
        "--- café 🦀\nThing has\n",
        "    field own Box(Int)\n",
        "    method(self ref Byte, value mut Byte, moved own Byte) Byte: return value\n",
        "ordinary Byte: 1\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let declarations = members(&file, 0);
    let field = declarations[0];
    assert!(field.parameters.is_none());
    let keyword = own_type(field).keyword.span_id;
    assert_eq!(file.span_table.get(keyword).extract(source), "own");
    let hit = file.node_at_offset(source.find("own").unwrap()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(_)
    ));
    let owners: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
    assert_eq!(owners, ["Thing", "field"]);
    for (word, expected) in [
        ("ref", Convention::Ref),
        ("mut", Convention::Mut),
        ("own Byte", Convention::Own),
    ] {
        let hit = file.node_at_offset(source.find(word).unwrap()).unwrap();
        let SyntaxNodeKind::Convention(convention) = hit.node.kind else {
            panic!("written parameter convention: {hit:?}");
        };
        assert_eq!(convention.value, expected);
        assert!(
            hit.ancestors
                .iter()
                .any(|node| { matches!(node.kind, SyntaxNodeKind::Parameter(_)) })
        );
        let owners: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
        assert_eq!(owners, ["Thing", "method"]);
    }
    assert!(binding(&file, 1).annotation.is_some());
    assert!(declarations[1].annotation.is_some());
}

fn annotation_form(annotation: &Annotation) -> &'static str {
    match annotation {
        Annotation::Expression(_) => "expression",
        Annotation::Refined { .. } => "refined",
        Annotation::Domain { .. } => "domain",
        Annotation::Predicate { .. } => "predicate",
        Annotation::Bound { .. } => "bound",
    }
}

#[test]
fn fields_receivers_and_method_returns_reuse_all_four_annotation_forms() {
    for (annotation, expected) in [
        ("Box(Int)", "expression"),
        ("Byte and self > 0", "refined"),
        ("in 0 to max and < max", "domain"),
        ("is >= 0 and < max", "predicate"),
    ] {
        let field_annotation = match expected {
            "expression" | "refined" => format!("own {annotation}"),
            _ => annotation.to_owned(),
        };
        let source = format!(
            "Thing has\n    field {field_annotation}\n    \
             method(self mut {annotation}) {annotation}: return self\n"
        );
        let file = parser::parse_strict(&source).expect(&source);
        let declarations = members(&file, 0);
        let receiver = &declarations[1].parameters.as_ref().unwrap().value[0].value;
        for (written, text) in [
            (
                declarations[0].annotation.as_ref().unwrap(),
                field_annotation.as_str(),
            ),
            (receiver.annotation.as_ref().unwrap(), annotation),
            (declarations[1].annotation.as_ref().unwrap(), annotation),
        ] {
            assert_eq!(annotation_form(&written.value), expected);
            assert_eq!(file.span_table.get(written.span_id).extract(&source), text);
        }
        assert_eq!(
            receiver.convention.as_ref().unwrap().value.keyword.value,
            Convention::Mut
        );
    }
    // Syntax does not decide whether mutating a Byte receiver is legal.
    assert!(
        parser::parse_strict("Thing has method(self mut in Byte) in Byte: return self\n")
            .is_ok()
    );
}

#[test]
fn old_member_domain_headers_recover_but_type_definitions_keep_is_in() {
    for header in [
        "Thing has field is in Byte",
        "Thing has method(self ref is in Byte): return self",
        "Thing has method(self) is in Byte: return self",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err());
        let diagnostic = output
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.span.extract(&source) == "is")
            .expect("noncanonical is diagnostic");
        assert!(diagnostic.help.as_deref().unwrap().contains("in"));
        let declarations = members(&output.file, 0);
        let written = if header.contains("ref is") {
            declarations[0].parameters.as_ref().unwrap().value[0]
                .value
                .annotation
                .as_ref()
                .unwrap()
        } else {
            declarations[0].annotation.as_ref().unwrap()
        };
        assert!(matches!(
            written.value,
            Annotation::Domain {
                is_keyword: Some(_),
                ..
            }
        ));
        assert_eq!(
            file_annotation_text(&output.file, written.span_id),
            "is in Byte"
        );
        assert_eq!(binding(&output.file, 1).name.value, "later");
    }
    let file = parser::parse_strict("Byte is in 0 to 255\n").unwrap();
    let Item::Type(declaration) = &file.items[0].value else {
        panic!("domain type");
    };
    assert!(matches!(declaration.body.value, TypeBody::Domain(_)));
}

fn file_annotation_text(file: &SourceFile, span: ast::SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn multiline_separators_and_nested_bodies_preserve_member_boundaries() {
    let source = concat!(
        "Thing has\n",
        "    field own Box(Int)\n",
        "    method(\n        self ref\n        value own in Byte,\n",
        "        other mut Int\n    ) in Byte:\n",
        "        local(x ref Int) Int:\n",
        "            return x\n",
        "        return local(value)\n",
        "    next own Int\n",
        "later := 2\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let declarations = members(&file, 0);
    assert_eq!(declarations.len(), 3);
    assert_eq!(declarations[2].name.value, "next");
    let method = declarations[1];
    assert_eq!(method.parameters.as_ref().unwrap().value.len(), 3);
    let Expr::Block(body) = &method.value.as_ref().unwrap().expression.value else {
        panic!("method block");
    };
    assert_eq!(body.statements.len(), 2);
    let Statement::Binding(local) = &body.statements[0].value else {
        panic!("nested local function");
    };
    assert_eq!(local.name.value, "local");
    assert!(matches!(body.statements[1].value, Statement::Return(_)));
    assert_eq!(binding(&file, 1).name.value, "later");
}

#[test]
fn malformed_and_deferred_conventions_are_diagnosed_without_losing_later_source() {
    for header in [
        "f(ref value Byte): return value",
        "f(mut value Byte): return value",
        "f(value own{arena} Byte): return value",
        "Thing has method(self ref mut): return self",
        "Thing has field own",
        "Thing has field own in Byte",
        "Thing has field own is >= 0",
        "Thing has field own is in Byte",
        "value := own Byte",
        "f(value own Byte: 1): return value",
        "Thing has Trait and\n    field Int",
        "Thing has qualified.method(self): return self",
    ] {
        let source = format!("{header}\nlater := 2\n");
        let output = parser::parse(&source).expect(&source);
        assert!(output.has_errors(), "{source}");
        assert!(parser::parse_strict(&source).is_err(), "{source}");
        let hit = output
            .file
            .node_at_offset(source.find("later").unwrap())
            .unwrap();
        assert!(
            matches!(hit.node.kind, SyntaxNodeKind::Name("later")),
            "{source}"
        );
    }
}

#[test]
fn owned_field_missing_annotation_is_a_hole_not_a_fabricated_type() {
    let source = "Thing has\n    field own\n    next Int\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let declarations = members(&output.file, 0);
    assert_eq!(declarations.len(), 2);
    let field = declarations[0];
    let expression = &own_type(field).target;
    assert_eq!(expression.value, Expr::Missing);
    assert!(output.file.span_table.get(expression.span_id).is_empty());
    assert_eq!(declarations[1].name.value, "next");
    assert_eq!(binding(&output.file, 1).name.value, "later");
}

#[test]
fn unannotated_defaults_keep_their_written_convention() {
    let file = parser::parse_strict("f(value own: 1): return value\n").unwrap();
    let parameter = &binding(&file, 0).parameters.as_ref().unwrap().value[0].value;
    assert_eq!(
        parameter.convention.as_ref().unwrap().value.keyword.value,
        Convention::Own
    );
    assert!(parameter.annotation.is_none());
    assert!(parameter.default.is_some());
}

#[test]
fn bare_and_constraints_preserve_absent_bases_without_fabricating_self_or_is() {
    let source = concat!(
        "Thing has\n",
        "    value and > 0\n",
        "    method(self mut and self.count > 0, x ref and > 0) and > 0: return x\n",
    );
    let file = parser::parse_strict(source).unwrap();
    let declarations = members(&file, 0);
    let parameters = &declarations[1].parameters.as_ref().unwrap().value;
    for annotation in [
        declarations[0].annotation.as_ref().unwrap(),
        parameters[0].value.annotation.as_ref().unwrap(),
        parameters[1].value.annotation.as_ref().unwrap(),
        declarations[1].annotation.as_ref().unwrap(),
    ] {
        let Annotation::Refined { base, refinements } = &annotation.value else {
            panic!("bare-and refinement");
        };
        assert!(base.is_none());
        assert_eq!(refinements.len(), 1);
        assert!(file_annotation_text(&file, annotation.span_id).starts_with("and "));
    }
    let source = "Thing has value own and > 0\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    assert!(parser::parse_strict(source).is_err());
    let declarations = members(&output.file, 0);
    let Annotation::Refined { base, refinements } =
        &declarations[0].annotation.as_ref().unwrap().value
    else {
        panic!("owned field retains missing base and constraint");
    };
    let Expr::PrefixedType(prefix) = &base.as_ref().unwrap().value else {
        panic!("own type retains its missing target");
    };
    assert_eq!(prefix.keyword.value, TypePrefixKind::Own);
    assert_eq!(prefix.target.value, Expr::Missing);
    assert!(output.file.span_table.get(prefix.target.span_id).is_empty());
    assert_eq!(refinements.len(), 1);
}

#[test]
fn member_annotations_and_bad_modes_obey_resource_budgets() {
    use parser::{ParseFailure, ParseLimit, ParseLimits, ParseOptions};

    for (source, limits, expected) in [
        (
            "Thing has field in - -limit to max\n",
            ParseLimits {
                expression_depth_max: 2,
                ..Default::default()
            },
            ParseLimit::ExpressionDepth,
        ),
        (
            "f(x ref mut): return x\ng(y own own): return y\n",
            ParseLimits {
                diagnostics_max: 1,
                ..Default::default()
            },
            ParseLimit::Diagnostics,
        ),
        (
            "Thing has field own Byte\n",
            ParseLimits {
                tokens_max: 3,
                ..Default::default()
            },
            ParseLimit::Tokens,
        ),
    ] {
        let options = ParseOptions {
            limits,
            ..Default::default()
        };
        let ParseFailure::LimitExceeded { limit, span, .. } =
            parser::parse_with_options(source, options).unwrap_err()
        else {
            panic!("resource refusal");
        };
        assert_eq!(limit, expected);
        assert!(source.get(span.to_range()).is_some());
    }
}

#[test]
fn bare_constraints_do_not_reclassify_ordinary_boolean_expression_statements() {
    let source = "left and right\nf() and flag\nmain:\n    left and right\n    return value\n";
    let file = parser::parse_strict(source).unwrap();
    for item in &file.items[..2] {
        let Item::Expression(expression) = &item.value else {
            panic!("ordinary expression, not a declaration");
        };
        assert!(matches!(
            expression.value,
            Expr::Binary { ref operator, .. } if operator.value == ast::BinaryOperator::And
        ));
    }
    let names: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(names, ["main"]);
}

#[test]
fn missing_owned_annotations_keep_eof_and_inline_member_boundaries() {
    let source = "Thing has\n    field own\n        -- hé 🦀\n        ";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let hit = output.file.node_at_offset(source.len()).unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::Expression(expression) if expression.value == Expr::Missing
    ));
    assert_eq!(hit.node.span.start(), source.len());
    assert_eq!(hit.declarations.last().unwrap().name, "field");
    let source = "Thing has field own, next Int\nlater := 2\n";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let fields = members(&output.file, 0);
    assert_eq!(fields.len(), 2);
    assert_eq!(own_type(fields[0]).target.value, Expr::Missing);
    assert_eq!(fields[1].name.value, "next");
}

#[test]
fn missing_call_delimiters_do_not_swallow_a_following_owned_field() {
    let source = "\
Thing has
    method(self ref):
        value := f(
    field own Box(Int)
    next and > 0
later := 2
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let fields = members(&output.file, 0);
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[1].name.value, "field");
    assert!(matches!(
        own_type(fields[1]).target.value,
        Expr::Call { .. }
    ));
    assert_eq!(fields[2].name.value, "next");
    let hit = output
        .file
        .node_at_offset(source.find("own").unwrap())
        .unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(_)
    ));
    assert_eq!(hit.declarations.last().unwrap().name, "field");
}

#[test]
fn a_later_orphan_close_cannot_hide_an_owned_member_or_close_its_has_scope() {
    let source = "\
Thing has
    broken := f(
    field own Int
)
    next Int
later := 1
";
    let output = parser::parse(source).unwrap();
    assert!(output.has_errors());
    let Item::Type(declaration) = &output.file.items[0].value else {
        panic!("has declaration survives");
    };
    let TypeBody::Has(entries) = &declaration.body.value else {
        panic!("has members");
    };
    let names: Vec<_> = entries
        .iter()
        .filter_map(|entry| match &entry.value {
            TypeMember::Binding(binding) => Some(binding.name.value.as_str()),
            TypeMember::Provision(_) | TypeMember::ProvisionApplied(_) | TypeMember::Error => None,
        })
        .collect();
    assert!(names.contains(&"field"), "{names:?}");
    assert!(names.contains(&"next"), "{names:?}");
    assert!(
        entries
            .iter()
            .any(|entry| matches!(entry.value, TypeMember::Error))
    );
    let hit = output
        .file
        .node_at_offset(source.find("own").unwrap())
        .unwrap();
    assert!(matches!(
        hit.node.kind,
        SyntaxNodeKind::TypePrefixKeyword(_)
    ));
    assert_eq!(hit.declarations.last().unwrap().name, "field");
    assert_eq!(binding(&output.file, 1).name.value, "later");
}

#[test]
fn balanced_parameter_conventions_are_not_recovered_as_owned_fields() {
    for source in [
        "read(\nvalue own Int\n) Int: return value\n",
        "Thing has\n    read(\n    value own Int\n    ) Int\n",
        concat!(
            "Thing has first(\nvalue own Int\n) Int, ",
            "second(\nvalue own Int\n) Int\n",
        ),
        concat!(
            "Thing(element) has first(\nvalue own element\n) element, ",
            "second(\nvalue own element\n) element\n",
        ),
    ] {
        let file = parser::parse_strict(source).expect(source);
        assert_eq!(file.items.len(), 1);
        for (offset, _) in source.match_indices("own") {
            let hit = file.node_at_offset(offset).unwrap();
            assert!(matches!(hit.node.kind, SyntaxNodeKind::Convention(_)));
        }
    }
}
