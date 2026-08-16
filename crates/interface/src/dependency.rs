use crate::{DeclarationRef, Fingerprint};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyEdge {
    PrivateFolded {
        value: Vec<u8>,
    },
    ReachableLocal {
        reference: DeclarationRef,
        fingerprint: Fingerprint,
    },
    External {
        reference: DeclarationRef,
        fingerprint: Fingerprint,
    },
}

pub fn public_closure_fingerprint(edges: &[DependencyEdge]) -> Fingerprint {
    let mut encoded: Vec<_> = edges
        .iter()
        .filter_map(|edge| match edge {
            DependencyEdge::PrivateFolded { .. } => None,
            DependencyEdge::ReachableLocal {
                reference,
                fingerprint,
            } => Some(encode_edge(0, reference, fingerprint)),
            DependencyEdge::External {
                reference,
                fingerprint,
            } => Some(encode_edge(1, reference, fingerprint)),
        })
        .collect();
    encoded.sort();
    encoded.dedup();
    let payload: Vec<_> = encoded.into_iter().flatten().collect();
    Fingerprint::from_domain(
        b"gin.interface.public-closure\0",
        crate::SCHEMA_VERSION,
        &payload,
    )
}

fn encode_edge(tag: u8, reference: &DeclarationRef, fingerprint: &Fingerprint) -> Vec<u8> {
    let mut output = vec![tag];
    reference.encode_identity(&mut output);
    output.extend_from_slice(&fingerprint.0);
    output
}
