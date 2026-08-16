use std::collections::{BTreeMap, BTreeSet};

use crate::{DeclarationRef, Fingerprint};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedDeclaration {
    pub reference: DeclarationRef,
    pub fingerprint: Fingerprint,
    pub dependencies: Vec<DeclarationRef>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexedInterfaceError {
    Duplicate(Box<DeclarationRef>),
    Missing(Box<DeclarationRef>),
    FingerprintMismatch(Box<DeclarationRef>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexedInterface {
    declarations: BTreeMap<DeclarationRef, IndexedDeclaration>,
}

impl IndexedInterface {
    pub fn new(
        declarations: impl IntoIterator<Item = IndexedDeclaration>,
    ) -> Result<Self, IndexedInterfaceError> {
        let mut indexed = BTreeMap::new();
        for mut declaration in declarations {
            declaration.dependencies.sort();
            declaration.dependencies.dedup();
            let reference = declaration.reference.clone();
            if indexed.insert(reference.clone(), declaration).is_some() {
                return Err(IndexedInterfaceError::Duplicate(Box::new(reference)));
            }
        }
        Ok(Self {
            declarations: indexed,
        })
    }

    pub fn decode_closure(
        &self,
        root: &DeclarationRef,
    ) -> Result<Vec<&IndexedDeclaration>, IndexedInterfaceError> {
        let mut pending = vec![root.clone()];
        let mut visited = BTreeSet::new();
        let mut decoded = Vec::new();
        while let Some(reference) = pending.pop() {
            if !visited.insert(reference.clone()) {
                continue;
            }
            let declaration = self
                .declarations
                .get(&reference)
                .ok_or_else(|| IndexedInterfaceError::Missing(Box::new(reference.clone())))?;
            if Fingerprint::from_bytes(&declaration.body) != declaration.fingerprint {
                return Err(IndexedInterfaceError::FingerprintMismatch(Box::new(
                    reference,
                )));
            }
            pending.extend(declaration.dependencies.iter().rev().cloned());
            decoded.push(declaration);
        }
        Ok(decoded)
    }
}
