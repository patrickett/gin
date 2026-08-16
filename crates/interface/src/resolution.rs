use std::collections::BTreeMap;

use crate::{DeclarationRef, Fingerprint};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ResolutionQuery {
    pub scope: String,
    pub namespace: String,
    pub name: String,
    pub shape: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionCandidate {
    pub reference: DeclarationRef,
    pub fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionIndex {
    buckets: BTreeMap<ResolutionQuery, Vec<ResolutionCandidate>>,
    scope_fingerprints: BTreeMap<String, Fingerprint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionDependency {
    query: ResolutionQuery,
    scope_fingerprint: Fingerprint,
    bucket_fingerprint: Option<Fingerprint>,
}

impl ResolutionIndex {
    pub fn new(entries: impl IntoIterator<Item = (ResolutionQuery, ResolutionCandidate)>) -> Self {
        let mut buckets: BTreeMap<_, Vec<_>> = BTreeMap::new();
        for (query, candidate) in entries {
            buckets.entry(query).or_default().push(candidate);
        }
        for candidates in buckets.values_mut() {
            candidates.sort_by(|left, right| left.reference.cmp(&right.reference));
            candidates.dedup_by(|left, right| left.reference == right.reference);
        }
        let mut scope_payloads: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();
        for (query, candidates) in &buckets {
            scope_payloads
                .entry(query.scope.clone())
                .or_default()
                .push(encode_bucket(query, candidates));
        }
        let scope_fingerprints = scope_payloads
            .into_iter()
            .map(|(scope, mut buckets)| {
                buckets.sort();
                let payload: Vec<_> = buckets.into_iter().flatten().collect();
                (
                    scope,
                    Fingerprint::from_domain(
                        b"gin.interface.resolution-scope\0",
                        crate::SCHEMA_VERSION,
                        &payload,
                    ),
                )
            })
            .collect();
        Self {
            buckets,
            scope_fingerprints,
        }
    }

    pub fn dependency(&self, query: ResolutionQuery) -> ResolutionDependency {
        let scope_fingerprint = self
            .scope_fingerprints
            .get(&query.scope)
            .copied()
            .unwrap_or_else(|| Fingerprint::from_bytes(query.scope.as_bytes()));
        let bucket_fingerprint = self.buckets.get(&query).map(|candidates| {
            Fingerprint::from_domain(
                b"gin.interface.candidate-bucket\0",
                crate::SCHEMA_VERSION,
                &encode_bucket(&query, candidates),
            )
        });
        ResolutionDependency {
            query,
            scope_fingerprint,
            bucket_fingerprint,
        }
    }

    pub fn dependency_is_current(&self, dependency: &ResolutionDependency) -> bool {
        self.dependency(dependency.query.clone()) == *dependency
    }
}

fn encode_bucket(query: &ResolutionQuery, candidates: &[ResolutionCandidate]) -> Vec<u8> {
    let mut output = Vec::new();
    for value in [&query.scope, &query.namespace, &query.name, &query.shape] {
        output.extend_from_slice(&(value.len() as u64).to_le_bytes());
        output.extend_from_slice(value.as_bytes());
    }
    for candidate in candidates {
        candidate.reference.encode_identity(&mut output);
        output.extend_from_slice(&candidate.fingerprint.0);
    }
    output
}
