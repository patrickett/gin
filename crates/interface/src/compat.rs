use std::collections::BTreeMap;

use crate::{EncodedIntegerValidity, PublicInterface};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compatibility {
    Unchanged,
    Compatible,
    Breaking,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidityPosition {
    Parameter,
    Return,
}

pub fn classify_interfaces(old: &PublicInterface, new: &PublicInterface) -> Compatibility {
    let old_count = old.declarations.len();
    let new_count = new.declarations.len();
    let old: BTreeMap<_, _> = old
        .declarations
        .iter()
        .map(|declaration| (&declaration.reference, declaration.fingerprint))
        .collect();
    let new: BTreeMap<_, _> = new
        .declarations
        .iter()
        .map(|declaration| (&declaration.reference, declaration.fingerprint))
        .collect();
    if old.len() != old_count || new.len() != new_count {
        return Compatibility::Unknown;
    }
    if old == new {
        return Compatibility::Unchanged;
    }
    if old
        .iter()
        .any(|(reference, fingerprint)| new.get(reference) != Some(fingerprint))
    {
        Compatibility::Breaking
    } else {
        Compatibility::Compatible
    }
}

pub fn classify_validity(
    old: &EncodedIntegerValidity,
    new: &EncodedIntegerValidity,
    position: ValidityPosition,
) -> Compatibility {
    if old.canonical_bytes() == new.canonical_bytes() {
        return Compatibility::Unchanged;
    }
    let (Some((old_min, old_max)), Some((new_min, new_max))) =
        (old.storage_hull_i128(), new.storage_hull_i128())
    else {
        return Compatibility::Unknown;
    };
    let old_contains_new = old_min <= new_min && new_max <= old_max;
    let new_contains_old = new_min <= old_min && old_max <= new_max;
    match position {
        ValidityPosition::Parameter if new_contains_old => Compatibility::Compatible,
        ValidityPosition::Return if old_contains_new => Compatibility::Compatible,
        ValidityPosition::Parameter if old_contains_new => Compatibility::Breaking,
        ValidityPosition::Return if new_contains_old => Compatibility::Breaking,
        _ => Compatibility::Unknown,
    }
}
