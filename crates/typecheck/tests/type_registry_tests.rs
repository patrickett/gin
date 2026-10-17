// Internal tests for this module are mounted from `src` via `#[cfg(test)]`.

use ast::Ty;
use ast::ty::{DeclarationKey, NamedTypeInstance, PackageInstanceKey, TyArg, TypeId};
use internment::Intern;

use typecheck::{TypeDeclarationSemantics, TypeRegistry};

#[test]
fn resolved_definition_substitutes_instance_arguments() {
    let parameter = Intern::from_ref("T");
    let name = Intern::from_ref("Wrapper");
    let id = TypeId { file: 1, name };
    let key = DeclarationKey {
        member: 0,
        package: PackageInstanceKey::workspace("test", "0"),
        module: Vec::new(),
        declaration: name,
    };
    let mut registry = TypeRegistry::default();
    registry.insert(
        id,
        TypeDeclarationSemantics {
            key,
            display_name: name,
            parameters: vec![parameter],
            definition: Ty::Record {
                name,
                fields: vec![(Intern::from_ref("value"), Box::new(Ty::Opaque(parameter)))],
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
    let instance = Ty::Named {
        instance: NamedTypeInstance {
            declaration: id,
            arguments: vec![(parameter, TyArg::Type(Box::new(Ty::Unit)))],
        },
        name,
    };

    assert_eq!(
        registry.resolved_definition_for_type(&instance),
        Ty::Record {
            name,
            fields: vec![(Intern::from_ref("value"), Box::new(Ty::Unit))],
            resolved_params: Some(vec![(parameter, TyArg::Type(Box::new(Ty::Unit)))]),
        }
    );
}

#[test]
fn stored_reference_tracking_is_independent_of_field_order() {
    let registry = TypeRegistry::default();
    for mutable in [false, true] {
        let reference = Ty::Ref {
            inner: Box::new(Ty::Unit),
            mutable,
        };
        assert!(!registry.storage_requires_borrow_tracking(&reference));
        for reverse in [false, true] {
            let mut fields = vec![
                (Intern::from_ref("borrow"), Box::new(reference.clone())),
                (
                    Intern::from_ref("storage"),
                    Box::new(Ty::OwnedStorage {
                        inner: Box::new(reference.clone()),
                    }),
                ),
            ];
            if reverse {
                fields.reverse();
            }
            let ty = Ty::Record {
                name: Intern::from_ref("Mixed"),
                fields,
                resolved_params: None,
            };
            assert!(
                registry.storage_requires_borrow_tracking(&ty),
                "mutable={mutable}, reverse={reverse}"
            );
        }
    }
}
