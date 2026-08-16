use ast::Ty;
use ast::integer::{
    IntegerDomain, IntegerInterpretation, IntegerRepresentationRule, IntegerValidity,
    NominalIntegerSemantics,
};
use ast::ty::{DeclarationKey, NamedTypeInstance, PackageInstanceKey, TypeId};
use i256::I256;
use internment::Intern;
use typecheck::reflect::{ReflectedIdentity, ReflectedSemantics, reflect_ty_to_reflected_type};
use typecheck::{TypeDeclarationSemantics, TypeRegistry};

fn nominal_integer(interpretation: IntegerInterpretation) -> (Ty, TypeRegistry) {
    let name = Intern::from_ref("Counter");
    let handle = TypeId { file: 0, name };
    let ty = Ty::Named {
        instance: NamedTypeInstance::new(handle),
        name,
    };
    let validity =
        IntegerValidity::new(IntegerDomain::bounded(I256::from(0), I256::from(255)).unwrap());
    let mut registry = TypeRegistry::default();
    registry.insert(
        handle,
        TypeDeclarationSemantics {
            key: DeclarationKey::new(PackageInstanceKey::workspace("test", "0"), vec![], name),
            display_name: name,
            parameters: Vec::new(),
            definition: Ty::anonymous_integer_for_width(8, false),
            fixed_array: None,
            reference: None,
            integer: Some(NominalIntegerSemantics {
                validity,
                representation_rule: IntegerRepresentationRule::InferFromValidity,
                interpretation,
            }),
        },
    );
    (ty, registry)
}

#[test]
fn nominal_integer_reflection_uses_registry_interpretation() {
    let (ty, registry) = nominal_integer(IntegerInterpretation::Signed);
    let reflected = reflect_ty_to_reflected_type(&ty, Some(&registry));

    assert!(matches!(reflected.identity, ReflectedIdentity::Named(_)));
    assert!(matches!(
        reflected.semantics,
        ReflectedSemantics::Integer {
            interpretation: Some(IntegerInterpretation::Signed),
            ..
        }
    ));
}

#[test]
fn anonymous_integer_reflection_has_no_persistent_interpretation() {
    let ty = Ty::anonymous_integer_for_width(8, true);
    let reflected = reflect_ty_to_reflected_type(&ty, None);

    assert_eq!(reflected.identity, ReflectedIdentity::Anonymous);
    assert!(matches!(
        reflected.semantics,
        ReflectedSemantics::Integer {
            interpretation: None,
            ..
        }
    ));
}
