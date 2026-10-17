use ast::Ty;
use ast::ty::{NamedTypeInstance, TypeId};
use internment::Intern;
use typecheck::TypeRegistry;
use typecheck::equality::{EqualityError, validate_with_registry};

fn unresolved(name: &str) -> Ty {
    let name = Intern::from_ref(name);
    Ty::Named {
        instance: NamedTypeInstance {
            declaration: TypeId { file: 0, name },
            arguments: Vec::new(),
        },
        name,
    }
}

#[test]
fn unresolved_nominal_equality_is_unsupported() {
    let ty = unresolved("Missing");
    assert_eq!(
        validate_with_registry(&ty, &ty, &TypeRegistry::default()),
        Err(EqualityError::UnsupportedType)
    );
}

#[test]
fn distinct_unresolved_nominal_types_cannot_be_compared() {
    assert_eq!(
        validate_with_registry(
            &unresolved("Left"),
            &unresolved("Right"),
            &TypeRegistry::default()
        ),
        Err(EqualityError::DifferentTypes)
    );
}

#[test]
fn unresolved_nominal_cannot_be_compared_to_a_structural_type() {
    let ty = unresolved("Missing");
    let registry = TypeRegistry::default();
    for (lhs, rhs) in [(&ty, &Ty::Unit), (&Ty::Unit, &ty)] {
        assert_eq!(
            validate_with_registry(lhs, rhs, &registry),
            Err(EqualityError::DifferentTypes)
        );
    }
}

fn record(name: &str, fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record {
        name: Intern::from_ref(name),
        fields: fields
            .into_iter()
            .map(|(name, ty)| (Intern::from_ref(name), Box::new(ty)))
            .collect(),
        resolved_params: None,
    }
}

fn register(registry: &mut TypeRegistry, ty: &Ty, definition: Ty) {
    let instance = ty.named_instance().unwrap();
    registry.insert(
        instance.declaration,
        typecheck::TypeDeclarationSemantics {
            key: ast::ty::DeclarationKey::new(
                ast::ty::PackageInstanceKey::workspace("equality", "0"),
                Vec::new(),
                instance.declaration.name,
            ),
            display_name: instance.declaration.name,
            parameters: instance.arguments.iter().map(|(name, _)| *name).collect(),
            definition,
            sequence_literal: None,
            private_members: Default::default(),
            private_fields: Default::default(),
            field_refinements: Default::default(),
            reference: None,
            integer: None,
            discriminant: None,
        },
    );
}

fn recursive(case: &str) -> (Ty, TypeRegistry) {
    let mut ty = unresolved("Node");
    let mut registry = TypeRegistry::default();
    let next = match case {
        "direct" => ty.clone(),
        "reference" => Ty::Ref {
            inner: Box::new(ty.clone()),
            mutable: false,
        },
        "mutual" => {
            let other = unresolved("Other");
            register(
                &mut registry,
                &other,
                record("Other", vec![("next", ty.clone())]),
            );
            other
        }
        "growing" => {
            let parameter = Intern::from_ref("element");
            let Ty::Named { instance, .. } = &mut ty else {
                unreachable!()
            };
            instance.arguments = vec![(parameter, ast::ty::TyArg::Type(Box::new(Ty::Unit)))];
            let mut next = ty.clone();
            let Ty::Named { instance, .. } = &mut next else {
                unreachable!()
            };
            instance.arguments = vec![(
                parameter,
                ast::ty::TyArg::Type(Box::new(Ty::Array {
                    elem: Box::new(Ty::Opaque(parameter)),
                    size: 2_i128.into(),
                })),
            )];
            next
        }
        "named-reference" => {
            register(&mut registry, &ty, Ty::Unit);
            let instance = ty.named_instance().unwrap();
            let mut declaration = registry.declaration(instance.declaration).unwrap().clone();
            declaration.reference = Some(typecheck::type_registry::ReferenceSemantics {
                pointee: ty.clone(),
                permission: typecheck::type_registry::ReferencePermission::Observe,
            });
            registry.insert(instance.declaration, declaration);
            return (ty, registry);
        }
        _ => panic!("unknown recursion case"),
    };
    register(&mut registry, &ty, record("Node", vec![("next", next)]));
    (ty, registry)
}

#[test]
fn recursive_record_equality_terminates_with_an_unsupported_diagnostic() {
    const CASE: &str = "GIN_TEST_RECURSIVE_EQUALITY_CASE";
    if let Ok(case) = std::env::var(CASE) {
        if case == "mismatched-sibling" {
            let (node, registry) = recursive("direct");
            let lhs = record("Pair", vec![("node", node.clone()), ("value", Ty::Unit)]);
            let rhs = record(
                "Pair",
                vec![
                    ("node", node),
                    ("value", Ty::anonymous_integer_for_width(8, false)),
                ],
            );
            assert_eq!(
                validate_with_registry(&lhs, &rhs, &registry),
                Err(EqualityError::DifferentTypes)
            );
            return;
        }
        let (ty, registry) = recursive(&case);
        assert_eq!(
            validate_with_registry(&ty, &ty, &registry),
            Err(EqualityError::UnsupportedType)
        );
        return;
    }
    let thread = std::thread::current();
    let mut failures = Vec::new();
    for case in [
        "direct",
        "reference",
        "mutual",
        "growing",
        "named-reference",
        "mismatched-sibling",
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", thread.name().unwrap(), "--nocapture"])
            .env(CASE, case)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        if !output.status.success() {
            failures.push(format!(
                "{case}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn repeated_acyclic_nominal_fields_support_equality() {
    let child = unresolved("Child");
    let ty = record(
        "Pair",
        vec![("left", child.clone()), ("right", child.clone())],
    );
    let mut registry = TypeRegistry::default();
    register(
        &mut registry,
        &child,
        record("Child", vec![("value", Ty::Unit)]),
    );
    assert_eq!(validate_with_registry(&ty, &ty, &registry), Ok(()));
}

fn applied(name: &str, argument: Ty) -> Ty {
    let mut ty = unresolved(name);
    let Ty::Named { instance, .. } = &mut ty else {
        unreachable!()
    };
    instance.arguments = vec![(
        Intern::from_ref("element"),
        ast::ty::TyArg::Type(Box::new(argument)),
    )];
    ty
}

#[test]
fn finite_nested_instances_of_one_declaration_support_equality() {
    let ty = applied("Box", applied("Box", Ty::Unit));
    let mut registry = TypeRegistry::default();
    register(
        &mut registry,
        &ty,
        record(
            "Box",
            vec![("value", Ty::Opaque(Intern::from_ref("element")))],
        ),
    );
    assert_eq!(validate_with_registry(&ty, &ty, &registry), Ok(()));
}

#[test]
fn nested_reference_instances_compare_their_actual_pointees() {
    let ty = applied("Reference", applied("Reference", Ty::Unit));
    let other = applied(
        "Reference",
        applied("Reference", Ty::anonymous_integer_for_width(8, false)),
    );
    let mut registry = TypeRegistry::default();
    register(&mut registry, &ty, Ty::Unit);
    let instance = ty.named_instance().unwrap();
    let mut declaration = registry.declaration(instance.declaration).unwrap().clone();
    declaration.reference = Some(typecheck::type_registry::ReferenceSemantics {
        pointee: Ty::Opaque(Intern::from_ref("element")),
        permission: typecheck::type_registry::ReferencePermission::Observe,
    });
    registry.insert(instance.declaration, declaration);
    assert_eq!(validate_with_registry(&ty, &ty, &registry), Ok(()));
    assert_eq!(
        validate_with_registry(&ty, &other, &registry),
        Err(EqualityError::DifferentTypes)
    );
}
