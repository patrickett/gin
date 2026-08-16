use interface::{AbiParameter, AbiSignature, AbiValidationError, Fingerprint, ParameterMode};

fn parameter(name: &str) -> AbiParameter {
    AbiParameter {
        name: name.to_string(),
        mode: ParameterMode::Own,
        default: None,
        receiver: false,
        foreign_slot: None,
    }
}

#[test]
fn declaration_reorder_does_not_change_canonical_abi() {
    let left = AbiSignature {
        parameters: vec![parameter("zeta"), parameter("alpha")],
        foreign: false,
    };
    let right = AbiSignature {
        parameters: vec![parameter("alpha"), parameter("zeta")],
        foreign: false,
    };

    assert_eq!(
        left.canonical_bytes().unwrap(),
        right.canonical_bytes().unwrap()
    );
}

#[test]
fn written_effect_order_precedes_canonical_slot_permutation() {
    let signature = AbiSignature {
        parameters: vec![parameter("alpha"), parameter("zeta")],
        foreign: false,
    };
    let mut effects = Vec::new();
    let written = ["zeta", "alpha"]
        .into_iter()
        .map(|name| {
            effects.push(name);
            (name.to_string(), name)
        })
        .collect();
    let slots = signature.permute_evaluated_arguments(written).unwrap();

    assert_eq!(effects, vec!["zeta", "alpha"]);
    assert_eq!(slots, vec!["alpha", "zeta"]);
}

#[test]
fn rename_default_and_addition_change_the_public_abi() {
    let base = AbiSignature {
        parameters: vec![parameter("value")],
        foreign: false,
    };
    let renamed = AbiSignature {
        parameters: vec![parameter("renamed")],
        foreign: false,
    };
    let mut defaulted_parameter = parameter("value");
    defaulted_parameter.default = Some(Fingerprint::from_bytes(b"default"));
    let defaulted = AbiSignature {
        parameters: vec![defaulted_parameter],
        foreign: false,
    };
    let added = AbiSignature {
        parameters: vec![parameter("value"), parameter("optional")],
        foreign: false,
    };

    let base = base.canonical_bytes().unwrap();
    assert_ne!(base, renamed.canonical_bytes().unwrap());
    assert_ne!(base, defaulted.canonical_bytes().unwrap());
    assert_ne!(base, added.canonical_bytes().unwrap());
}

#[test]
fn foreign_abi_requires_a_complete_injective_name_to_slot_map() {
    let mut first = parameter("first");
    first.foreign_slot = Some(0);
    let missing = AbiSignature {
        parameters: vec![first.clone(), parameter("second")],
        foreign: true,
    };
    let mut duplicate = parameter("second");
    duplicate.foreign_slot = Some(0);
    let duplicated = AbiSignature {
        parameters: vec![first, duplicate],
        foreign: true,
    };

    assert_eq!(
        missing.canonical_bytes(),
        Err(AbiValidationError::MissingForeignSlot("second".to_string()))
    );
    assert_eq!(
        duplicated.canonical_bytes(),
        Err(AbiValidationError::DuplicateForeignSlot(0))
    );
}
