//! Source syntax/query evidence only, not checked core-library capability equivalence.
//! These nominal declarations supply no automatic owner, default, or initialization facts.

use ast::{
    Annotation, Argument, Binding, Expr, Expression, Item, SourceFile, Statement, SyntaxNodeKind,
    TypeBody, TypeMember, TypePrefixKind,
};

fn expression_shape(expression: &Expression, renamed: &str) -> Vec<String> {
    let mut pending = vec![expression];
    let mut shape = Vec::new();
    for _ in 0..64 {
        let Some(expression) = pending.pop() else {
            return shape;
        };
        match &expression.value {
            Expr::Name(name) => {
                let name = if name == renamed { "$nominal" } else { name };
                shape.push(format!("name:{name}"));
            }
            Expr::Call { callee, arguments } => {
                shape.push(format!("call:{}", arguments.value.len()));
                assert_eq!(arguments.value.len(), 1);
                let Argument::Positional(argument) = &arguments.value[0].value else {
                    panic!("expected positional type application");
                };
                pending.push(argument);
                pending.push(callee);
            }
            Expr::PrefixedType(prefix) => {
                assert!(matches!(
                    prefix.keyword.value,
                    TypePrefixKind::Ref | TypePrefixKind::Mut
                ));
                shape.push(format!("prefix:{:?}", prefix.keyword.value));
                pending.push(&prefix.target);
            }
            Expr::Member { base, name } => {
                shape.push(format!("member:{}", name.value));
                pending.push(base);
            }
            Expr::Block(block) => {
                assert_eq!(block.statements.len(), 1);
                let Statement::Return(returned) = &block.statements[0].value else {
                    panic!("expected explicit return");
                };
                assert!(returned.value.is_some());
                shape.push("block:return:1".into());
                pending.push(returned.value.as_ref().unwrap());
            }
            other => panic!("unexpected fixture expression: {other:?}"),
        }
    }
    panic!("fixture expression exceeds bounded traversal");
}

fn annotation_shape(annotation: &Annotation, renamed: &str) -> Vec<String> {
    let Annotation::Expression(expression) = annotation else {
        panic!("expected written expression annotation");
    };
    expression_shape(expression, renamed)
}

fn binding_shape(binding: &Binding, renamed: &str) -> Vec<String> {
    assert!(binding.convention.is_none());
    let mut shape = vec![
        format!("binding:{}", binding.name.value),
        "convention:none".into(),
    ];
    match &binding.parameters {
        None => shape.push("parameters:none".into()),
        Some(parameters) => {
            assert!(parameters.value.len() <= 1);
            shape.push(format!("parameters:{}", parameters.value.len()));
            for parameter in &parameters.value {
                let parameter = &parameter.value;
                assert!(parameter.convention.is_none());
                assert!(parameter.default.is_none());
                shape.push(format!("parameter:{}:none:none", parameter.name.value));
                match &parameter.annotation {
                    Some(annotation) => shape.extend(annotation_shape(&annotation.value, renamed)),
                    None => shape.push("annotation:none".into()),
                }
            }
        }
    }
    shape.extend(annotation_shape(
        &binding.annotation.as_ref().unwrap().value,
        renamed,
    ));
    match &binding.value {
        None => shape.push("value:none".into()),
        Some(value) => {
            shape.push(format!("operator:{:?}", value.operator.value));
            shape.extend(expression_shape(&value.expression, renamed));
        }
    }
    shape
}

fn file_shape(file: &SourceFile, renamed: &str) -> Vec<String> {
    assert!(file.items.len() <= 8);
    let mut shape = Vec::new();
    for item in &file.items {
        match &item.value {
            Item::Type(declaration) => {
                let name = &declaration.name.value;
                let name = if name == renamed { "$nominal" } else { name };
                shape.push(format!("type:{name}"));
                match &declaration.parameters {
                    None => shape.push("parameters:none".into()),
                    Some(parameters) => {
                        assert_eq!(parameters.value.len(), 1);
                        let parameter = &parameters.value[0].value;
                        assert!(parameter.convention.is_none());
                        assert!(parameter.annotation.is_none());
                        assert!(parameter.default.is_none());
                        shape.push(format!("parameter:{}:none:none:none", parameter.name.value));
                    }
                }
                match &declaration.body.value {
                    TypeBody::Has(members) => {
                        assert!(members.len() <= 4);
                        shape.push(format!("has:{}", members.len()));
                        for member in members {
                            let TypeMember::Binding(binding) = &member.value else {
                                panic!("expected ordinary member");
                            };
                            shape.extend(binding_shape(binding, renamed));
                        }
                    }
                    TypeBody::Is(expression) => {
                        shape.push("is".into());
                        shape.extend(expression_shape(expression, renamed));
                    }
                    other => panic!("unexpected type body: {other:?}"),
                }
            }
            Item::Binding(binding) => shape.extend(binding_shape(binding, renamed)),
            other => panic!("unexpected fixture item: {other:?}"),
        }
    }
    shape
}

fn library_source(name: &str, comment: &str) -> String {
    format!(
        "-- {comment}
{name}(element) has
    memory MemoryToken(element)
    capacity Count(element)
    initialized Count(element)
    read(self) Count(element): return self.initialized
forward(value {name}(Item)) {name}(Item): return value
"
    )
}

fn assert_query(file: &SourceFile, offset: usize, name: &str, owners: &[&str]) {
    let hit = file.node_at_offset(offset).unwrap();
    assert!(matches!(hit.node.kind, SyntaxNodeKind::Name(found) if found == name));
    assert_eq!(hit.node.span.to_range(), offset..offset + name.len());
    assert_eq!(hit.node.span.extract(&file.text), name);
    let actual: Vec<_> = hit.declarations.iter().map(|owner| owner.name).collect();
    assert_eq!(actual, owners);
}

#[test]
fn nominal_library_surface_is_name_neutral_with_exact_byte_queries() {
    let baseline = library_source("Region", "plain");
    let baseline = parser::parse_strict(&baseline).unwrap();
    let expected = file_shape(&baseline, "Region");
    for (name, comment) in [
        ("Region", "hé 🦀"),
        ("ArenaBlock", "different length"),
        ("LocalBuffer", "日本語 🦀 café"),
    ] {
        let source = library_source(name, comment);
        let file = parser::parse_strict(&source).unwrap();
        assert_eq!(file_shape(&file, name), expected);
        assert_query(&file, source.find(name).unwrap(), name, &[name]);
        assert_query(
            &file,
            source.find("memory").unwrap(),
            "memory",
            &[name, "memory"],
        );
        assert_query(
            &file,
            source.find("MemoryToken").unwrap(),
            "MemoryToken",
            &[name, "memory"],
        );
        assert_query(
            &file,
            source.rfind("initialized").unwrap(),
            "initialized",
            &[name, "read"],
        );
        assert_query(&file, source.rfind(name).unwrap(), name, &["forward"]);
    }
    let changed = library_source("Region", "plain").replace("MemoryToken", "OtherToken");
    let changed = parser::parse_strict(&changed).unwrap();
    assert_ne!(file_shape(&changed, "Region"), expected);
}

#[test]
fn composed_type_calls_and_prefixes_are_written_shapes_not_inferred_conventions() {
    let expected = [
        vec![
            "call:1",
            "name:$nominal",
            "call:1",
            "name:$nominal",
            "name:Item",
        ],
        vec!["call:1", "name:$nominal", "prefix:Ref", "name:Item"],
        vec!["prefix:Ref", "call:1", "name:$nominal", "name:Item"],
        vec!["prefix:Mut", "call:1", "name:$nominal", "name:Item"],
        vec!["name:Alias"],
        vec![
            "call:1",
            "name:Wrapper",
            "call:1",
            "name:$nominal",
            "name:Item",
        ],
    ];
    for name in ["Region", "ArenaBlock", "LocalBuffer"] {
        let forms = [
            format!("{name}({name}(Item))"),
            format!("{name}(ref Item)"),
            format!("ref {name}(Item)"),
            format!("mut {name}(Item)"),
            "Alias".into(),
            format!("Wrapper({name}(Item))"),
        ];
        for (form, expected) in forms.iter().zip(&expected) {
            let source = format!(
                "-- café 🦀\nAlias is {name}(Item)\nWrapper(element) has value element\n\
                 forward(value Alias) {form}: return value\n"
            );
            let file = parser::parse_strict(&source).unwrap();
            let baseline_source = source.replace(name, "Region");
            let baseline = parser::parse_strict(&baseline_source).unwrap();
            assert_eq!(file_shape(&file, name), file_shape(&baseline, "Region"));
            let Item::Binding(binding) = &file.items[2].value else {
                panic!("ordinary forwarding binding");
            };
            let shape = binding_shape(binding, name);
            assert!(shape.contains(&"convention:none".into()));
            let annotation = &binding.annotation.as_ref().unwrap().value;
            assert_eq!(annotation_shape(annotation, name), *expected);
            let parameter = &binding.parameters.as_ref().unwrap().value[0].value;
            assert_eq!(
                annotation_shape(&parameter.annotation.as_ref().unwrap().value, name),
                ["name:Alias"]
            );
            assert_query(&file, source.rfind("value").unwrap(), "value", &["forward"]);
        }
    }
}
