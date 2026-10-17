use ast::ty::{DeclarationKey, NamedTypeInstance, PackageInstanceKey, TypeId};
use ast::{NormalExpr, Ty};
use internment::Intern;
use typecheck::representation::{RepresentationContext, has_symbolic_fixed_array_length};
use typecheck::{TypeDeclarationSemantics, TypeRegistry};

fn named(name: &str) -> Ty {
    let name = Intern::from_ref(name);
    Ty::Named {
        instance: NamedTypeInstance::new(TypeId { file: 0, name }),
        name,
    }
}

fn insert(registry: &mut TypeRegistry, name: &str, fields: Vec<(&str, Ty)>) {
    let name = Intern::from_ref(name);
    registry.insert(
        TypeId { file: 0, name },
        TypeDeclarationSemantics {
            key: DeclarationKey::new(PackageInstanceKey::workspace("test", "0"), vec![], name),
            display_name: name,
            parameters: Vec::new(),
            definition: Ty::Record {
                name,
                fields: fields
                    .into_iter()
                    .map(|(name, ty)| (Intern::from_ref(name), Box::new(ty)))
                    .collect(),
                resolved_params: None,
            },
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

fn pointer(ty: Ty) -> Ty {
    Ty::Ptr {
        inner: Box::new(ty),
    }
}

#[test]
fn symbolic_length_query_terminates_for_recursive_pointer_records() {
    let mut registry = TypeRegistry::default();
    insert(
        &mut registry,
        "Node",
        vec![("next", pointer(named("Node")))],
    );
    let context = RepresentationContext::new(Some(&registry));
    assert!(!context.has_symbolic_fixed_array_length(&named("Node")));
}

#[test]
fn symbolic_length_query_terminates_for_mutually_recursive_records() {
    let mut registry = TypeRegistry::default();
    insert(
        &mut registry,
        "First",
        vec![("next", pointer(named("Second")))],
    );
    insert(
        &mut registry,
        "Second",
        vec![("next", pointer(named("First")))],
    );
    assert!(!has_symbolic_fixed_array_length(
        &named("First"),
        Some(&registry)
    ));
}

#[test]
fn symbolic_length_query_checks_fields_after_a_recursive_edge() {
    let mut registry = TypeRegistry::default();
    insert(
        &mut registry,
        "Node",
        vec![
            ("next", pointer(named("Node"))),
            (
                "data",
                Ty::Array {
                    elem: Box::new(Ty::Unit),
                    size: NormalExpr::Var(Intern::from_ref("n")),
                },
            ),
        ],
    );
    assert!(has_symbolic_fixed_array_length(
        &named("Node"),
        Some(&registry)
    ));
}

#[test]
fn symbolic_length_query_visits_nonrecursive_sibling_declarations() {
    let mut registry = TypeRegistry::default();
    insert(&mut registry, "Empty", vec![]);
    insert(
        &mut registry,
        "Data",
        vec![(
            "data",
            Ty::Array {
                elem: Box::new(Ty::Unit),
                size: NormalExpr::Var(Intern::from_ref("n")),
            },
        )],
    );
    insert(
        &mut registry,
        "Pair",
        vec![("first", named("Empty")), ("second", named("Data"))],
    );
    assert!(has_symbolic_fixed_array_length(
        &named("Pair"),
        Some(&registry)
    ));
}
