use std::collections::BTreeMap;

use crate::PublicInterface;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compatibility {
    Unchanged,
    Compatible,
    Breaking,
    Unknown,
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
