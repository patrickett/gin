use ast::Ty;
use ast::integer::{
    IntegerDomain, IntegerInterpretation, IntegerRepresentationRule, IntegerValidity,
    NominalIntegerSemantics,
};
use ast::ty::{DeclarationKey, NamedTypeInstance, PackageInstanceKey, TypeId};
use i256::I256;
use internment::Intern;
use typecheck::representation::{Repr, RepresentationError};
use typecheck::{TypeDeclarationSemantics, TypeRegistry};

fn nominal_integer(name: &'static str, min: I256, max: I256) -> (Ty, TypeRegistry) {
    let name = Intern::from_ref(name);
    let handle = TypeId { file: 0, name };
    let instance = NamedTypeInstance::new(handle);
    let ty = Ty::Named { instance, name };
    let mut registry = TypeRegistry::default();
    registry.insert(
        handle,
        TypeDeclarationSemantics {
            key: DeclarationKey::new(PackageInstanceKey::workspace("test", "0"), vec![], name),
            display_name: name,
            parameters: Vec::new(),
            definition: Ty::Unit,
            fixed_array: None,
            reference: None,
            integer: Some(NominalIntegerSemantics {
                validity: IntegerValidity::new(IntegerDomain::bounded(min, max).unwrap()),
                representation_rule: IntegerRepresentationRule::InferFromValidity,
                interpretation: IntegerInterpretation::Unsigned,
            }),
        },
    );
    (ty, registry)
}

#[test]
fn nominal_integer_representation_comes_from_registry_metadata() {
    let (ty, registry) = nominal_integer("Window", I256::from(250), I256::from(260));
    assert_eq!(
        Repr::derive(&ty, Some(&registry)),
        Ok(Repr::Bits { width: 9 })
    );
}

#[test]
fn nominal_integer_runtime_width_is_validated_before_representation() {
    let max = (I256::from(1) << 199) - I256::from(1);
    let (ty, registry) = nominal_integer("Wide", I256::from(0), max);
    assert!(matches!(
        Repr::derive(&ty, Some(&registry)),
        Err(RepresentationError::InvalidInteger {
            code: "type-integer-runtime-width-unsupported",
            ..
        })
    ));
}

#[test]
fn two_alternative_structural_result_family_uses_one_bit_sum_discriminant() {
    let ty = Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::Structural(ast::OperatorRole::Less),
        alternatives: vec![
            ast::ResultAlternative {
                label: Intern::from_ref("False"),
                proposition: None,
            },
            ast::ResultAlternative {
                label: Intern::from_ref("True"),
                proposition: None,
            },
        ],
    };
    let representation = Repr::derive(&ty, None).expect("result-family representation");
    let layout = typecheck::layout::TargetLayout::x86_64()
        .layout_repr(&representation)
        .expect("result-family layout");

    assert!(matches!(
        layout.kind,
        typecheck::layout::LayoutKind::Sum {
            discriminant: Repr::Bits { width: 1 },
            ..
        }
    ));
}
