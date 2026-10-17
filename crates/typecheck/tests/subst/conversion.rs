use super::*;

#[test]
fn type_reference_conversion_requires_every_nominal_argument() {
    let name = Intern::from_ref("Container");
    let named = |arguments: Vec<TyArg>| Ty::Named {
        name,
        instance: ast::ty::NamedTypeInstance::new(ast::ty::TypeId { file: 0, name })
            .with_arguments(
                arguments
                    .into_iter()
                    .enumerate()
                    .map(|(index, argument)| (Intern::new(format!("argument{index}")), argument))
                    .collect(),
            ),
    };
    let unit = TyArg::Type(Box::new(Ty::Unit));
    assert_eq!(
        type_reference_for_ty(&named(vec![unit.clone(), unit.clone()])),
        Some(ast::TypeReference::Application {
            name,
            arguments: vec![ast::TypeReference::Unit; 2]
        })
    );
    assert_eq!(
        type_reference_for_ty(&named(Vec::new())),
        Some(ast::TypeReference::Application {
            name,
            arguments: Vec::new()
        })
    );
    let element = Intern::from_ref("Element");
    assert_eq!(
        type_reference_for_ty(&named(vec![
            TyArg::Type(Box::new(Ty::Opaque(element))),
            TyArg::Type(Box::new(Ty::Ptr {
                inner: Box::new(Ty::Unit),
            })),
        ])),
        Some(ast::TypeReference::Application {
            name,
            arguments: vec![
                ast::TypeReference::Nominal(element),
                ast::TypeReference::Pointer(Box::new(ast::TypeReference::Unit)),
            ],
        })
    );
    for unsupported in [
        TyArg::Const(3_i128.into()),
        TyArg::Type(Box::new(Ty::OwnedStorage {
            inner: Box::new(Ty::Unit),
        })),
    ] {
        for arguments in [
            vec![unsupported.clone(), unit.clone()],
            vec![unit.clone(), unsupported.clone()],
        ] {
            let ty = named(arguments);
            assert_eq!(type_reference_for_ty(&ty), None);
            assert_eq!(
                type_reference_for_ty(&Ty::Ptr {
                    inner: Box::new(ty.clone())
                }),
                None
            );
            let parameter = Intern::from_ref("T");
            let reference = ast::TypeReference::Nominal(parameter);
            let mut substitution = DepSubst::new();
            substitution.types.insert(parameter, ty.clone());
            assert_eq!(substitution.apply_to_type_reference(&reference), reference);
            assert_eq!(
                type_reference_for_ty(&named(vec![TyArg::Type(Box::new(ty))])),
                None
            );
        }
    }
}
