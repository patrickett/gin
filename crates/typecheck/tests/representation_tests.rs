use ast::Ty;
use ast::integer::{
    IntegerDomain, IntegerInterpretation, IntegerRepresentationRule, IntegerValidity,
    NominalIntegerSemantics,
};
use ast::ty::{DeclarationKey, NamedTypeInstance, PackageInstanceKey, TypeId};
use diagnostic::DiagnosticCode;
use diagnostic::code::{IntegerCode, TypeCode};
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
            sequence_literal: None,
            private_members: Default::default(),
            private_fields: Default::default(),
            field_refinements: Default::default(),

            reference: None,
            discriminant: None,
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
            code: DiagnosticCode::Type(TypeCode::Integer(IntegerCode::RuntimeWidthUnsupported)),
            ..
        })
    ));
}

#[test]
fn two_alternative_local_result_family_uses_one_bit_sum_discriminant() {
    let ty = Ty::ResultFamily {
        owner: ast::ResultFamilyOwner::LocalCallable(ast::BinderId::new(
            0,
            ast::BinderOwner::Expression(0),
        )),
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

#[test]
fn propositions_use_one_bit_without_a_result_family() {
    let ty = Ty::Proposition(Box::new(ast::ProofProposition::Compare {
        left: ast::ProofTerm::Name(Intern::from_ref("value")),
        relation: ast::ProofRelation::Less,
        right: ast::ProofTerm::Value(16.into()),
    }));
    assert_eq!(Repr::derive(&ty, None), Ok(Repr::Bits { width: 1 }));
}

#[test]
fn uniform_named_sequences_have_an_array_representation() {
    let ty = Ty::Sequence(vec![
        ast::ty::SequenceTypeElement {
            name: Some(Intern::from_ref("first")),
            ty: Box::new(Ty::anonymous_integer_for_width(8, false)),
        },
        ast::ty::SequenceTypeElement {
            name: Some(Intern::from_ref("second")),
            ty: Box::new(Ty::anonymous_integer_for_width(8, false)),
        },
    ]);

    assert!(matches!(
        Repr::derive(&ty, None),
        Ok(Repr::Array { length: 2, .. })
    ));
}
