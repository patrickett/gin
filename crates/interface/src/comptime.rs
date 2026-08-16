use std::collections::{HashMap, HashSet};

use crate::{DeclarationRef, Fingerprint, SCHEMA_VERSION};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComptimeImage {
    pub target: String,
    pub image_fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComptimeProgramNode {
    pub declaration: Option<DeclarationRef>,
    pub semantic_body: Vec<u8>,
    pub callees: Vec<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComptimeImageRecovery {
    Loaded(ComptimeImage),
    Rebuilt {
        image: ComptimeImage,
        code: &'static str,
    },
    Unavailable {
        code: &'static str,
    },
}

impl ComptimeImage {
    pub fn from_resolved_slice(
        target: impl Into<String>,
        nodes: &[ComptimeProgramNode],
        entries: &[usize],
    ) -> Option<Self> {
        let mut entries = entries.to_vec();
        entries.sort_by(|left, right| {
            nodes
                .get(*left)
                .and_then(|node| node.declaration.as_ref())
                .cmp(&nodes.get(*right).and_then(|node| node.declaration.as_ref()))
        });
        let mut encoder = ImageEncoder {
            nodes,
            private_ids: HashMap::new(),
            emitted: HashSet::new(),
            bytes: Vec::new(),
        };
        encoder.bytes.extend_from_slice(b"gin.comptime-image\0");
        encoder
            .bytes
            .extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        encode_u32(&mut encoder.bytes, entries.len())?;
        for entry in entries {
            let declaration = nodes.get(entry)?.declaration.as_ref()?;
            declaration.encode_identity(&mut encoder.bytes);
            encoder.visit(entry)?;
        }
        Some(Self {
            target: target.into(),
            image_fingerprint: Fingerprint::from_bytes(&encoder.bytes),
        })
    }
}

pub fn recover_comptime_image(
    cached: Option<ComptimeImage>,
    target: &str,
    expected: Fingerprint,
    rebuilt: Option<ComptimeImage>,
) -> ComptimeImageRecovery {
    let failure = match cached {
        None => "interface-comptime-image-missing",
        Some(image) if image.target != target => "interface-comptime-image-target-mismatch",
        Some(image) if image.image_fingerprint != expected => {
            "interface-comptime-image-incompatible"
        }
        Some(image) => return ComptimeImageRecovery::Loaded(image),
    };
    rebuilt.map_or(
        ComptimeImageRecovery::Unavailable { code: failure },
        |image| ComptimeImageRecovery::Rebuilt {
            image,
            code: failure,
        },
    )
}

struct ImageEncoder<'a> {
    nodes: &'a [ComptimeProgramNode],
    private_ids: HashMap<usize, u32>,
    emitted: HashSet<usize>,
    bytes: Vec<u8>,
}

impl ImageEncoder<'_> {
    fn visit(&mut self, index: usize) -> Option<()> {
        let node = self.nodes.get(index)?;
        if node.declaration.is_none() {
            let next = u32::try_from(self.private_ids.len()).ok()?;
            let id = *self.private_ids.entry(index).or_insert(next);
            self.bytes.push(0);
            self.bytes.extend_from_slice(&id.to_le_bytes());
        } else {
            self.bytes.push(1);
            node.declaration.as_ref()?.encode_identity(&mut self.bytes);
        }
        if !self.emitted.insert(index) {
            return Some(());
        }
        encode_bytes(&mut self.bytes, &node.semantic_body)?;
        encode_u32(&mut self.bytes, node.callees.len())?;
        for callee in &node.callees {
            self.visit(*callee)?;
        }
        Some(())
    }
}

fn encode_u32(bytes: &mut Vec<u8>, value: usize) -> Option<()> {
    bytes.extend_from_slice(&u32::try_from(value).ok()?.to_le_bytes());
    Some(())
}

fn encode_bytes(bytes: &mut Vec<u8>, value: &[u8]) -> Option<()> {
    encode_u32(bytes, value.len())?;
    bytes.extend_from_slice(value);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public(name: &str) -> DeclarationRef {
        DeclarationRef::Subject {
            module_path: "pkg".into(),
            declaration_path: name.into(),
        }
    }

    #[test]
    fn image_digest_ignores_discovery_order_and_private_helper_names() {
        let first = vec![
            ComptimeProgramNode {
                declaration: Some(public("entry")),
                semantic_body: b"call".to_vec(),
                callees: vec![1],
            },
            ComptimeProgramNode {
                declaration: None,
                semantic_body: b"add-one".to_vec(),
                callees: vec![],
            },
        ];
        let reordered = vec![
            ComptimeProgramNode {
                declaration: None,
                semantic_body: b"add-one".to_vec(),
                callees: vec![],
            },
            ComptimeProgramNode {
                declaration: Some(public("entry")),
                semantic_body: b"call".to_vec(),
                callees: vec![0],
            },
        ];

        let first = ComptimeImage::from_resolved_slice("x86", &first, &[0]).unwrap();
        let reordered = ComptimeImage::from_resolved_slice("x86", &reordered, &[1]).unwrap();
        assert_eq!(first.image_fingerprint, reordered.image_fingerprint);
    }

    #[test]
    fn image_digest_changes_when_private_helper_behavior_changes() {
        let make = |body: &[u8]| {
            vec![
                ComptimeProgramNode {
                    declaration: Some(public("entry")),
                    semantic_body: b"call".to_vec(),
                    callees: vec![1],
                },
                ComptimeProgramNode {
                    declaration: None,
                    semantic_body: body.to_vec(),
                    callees: vec![],
                },
            ]
        };
        let first = make(b"add-one");
        let second = make(b"add-two");

        assert_ne!(
            ComptimeImage::from_resolved_slice("x86", &first, &[0])
                .unwrap()
                .image_fingerprint,
            ComptimeImage::from_resolved_slice("x86", &second, &[0])
                .unwrap()
                .image_fingerprint,
        );
    }

    #[test]
    fn missing_or_incompatible_image_rebuilds_only_when_source_is_available() {
        let expected = Fingerprint::from_bytes(b"expected");
        let rebuilt = ComptimeImage {
            target: "x86".into(),
            image_fingerprint: expected,
        };
        assert!(matches!(
            recover_comptime_image(None, "x86", expected, Some(rebuilt.clone())),
            ComptimeImageRecovery::Rebuilt {
                code: "interface-comptime-image-missing",
                ..
            }
        ));
        assert_eq!(
            recover_comptime_image(None, "x86", expected, None),
            ComptimeImageRecovery::Unavailable {
                code: "interface-comptime-image-missing"
            }
        );
        let stale = ComptimeImage {
            target: "x86".into(),
            image_fingerprint: Fingerprint::from_bytes(b"stale"),
        };
        assert!(matches!(
            recover_comptime_image(Some(stale), "x86", expected, Some(rebuilt)),
            ComptimeImageRecovery::Rebuilt {
                code: "interface-comptime-image-incompatible",
                ..
            }
        ));
    }
}
