//! Canonical public interface primitives.

use std::fmt;

mod abi;
mod artifact;
mod codec;
mod compat;
mod comptime;
mod dependency;
mod encoding;
mod fingerprint;
mod identity;
mod public;
mod publish;
mod resolution;
mod schema;
mod target;
mod validation;

pub use abi::{AbiParameter, AbiSignature, AbiValidationError, ParameterMode};
pub use artifact::{ArtifactExpectation, ArtifactRecovery, recover_artifact};
pub use codec::{decode as decode_interface, encode as encode_interface};
pub use compat::{Compatibility, ValidityPosition, classify_interfaces, classify_validity};
pub use comptime::{
    ComptimeImage, ComptimeImageRecovery, ComptimeProgramNode, recover_comptime_image,
};
pub use dependency::{DependencyEdge, public_closure_fingerprint};
pub use encoding::{
    EncodedIntegerValidity, EncodedResultAlternative, EncodedResultFamily,
    EncodedResultFamilyOwner, IntegerExpr, IntegerPredicate, SemanticDecodeError, SignedInteger,
    StructuralOperator,
};
pub use fingerprint::Fingerprint;
pub use identity::{DeclarationRef, PackageInstanceId};
pub use public::{IndexedDeclaration, IndexedInterface, IndexedInterfaceError};
pub use publish::InterfacePublication;
pub use resolution::{ResolutionCandidate, ResolutionDependency, ResolutionIndex, ResolutionQuery};
pub use schema::{ARTIFACT_SECTIONS, IntegerExpressionTag, IntegerPredicateTag};
pub use target::{
    CachedTargetRealization, TargetProfile, TargetRealization, TargetRealizationExpectation,
    TargetRealizationRecovery, recover_target_realization,
};
pub use validation::{InterfaceValidationError, validate as validate_interface};

pub const SCHEMA_VERSION: u32 = 2;
pub const MAGIC: &[u8] = b"GINIFACE";

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum InterfaceDecodeError {
    ShortInput,
    CorruptHeader,
    UnsupportedSchema(u32),
}

impl fmt::Display for InterfaceDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShortInput => write!(f, "input too short"),
            Self::CorruptHeader => write!(f, "malformed interface artifact"),
            Self::UnsupportedSchema(schema) => write!(f, "unsupported schema version: {schema}"),
        }
    }
}

impl std::error::Error for InterfaceDecodeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceDeclaration {
    pub reference: DeclarationRef,
    pub fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicInterface {
    pub schema_version: u32,
    pub subject: PackageInstanceId,
    pub semantic_surface_fingerprint: Fingerprint,
    pub public_closure_fingerprint: Fingerprint,
    pub declarations: Vec<InterfaceDeclaration>,
    pub target_profiles: Vec<TargetProfile>,
    pub target_realizations: Vec<TargetRealization>,
    pub comptime_images: Vec<ComptimeImage>,
}

impl PublicInterface {
    pub fn base_fingerprint(&self) -> Fingerprint {
        let mut input = Vec::new();
        input.extend_from_slice(b"gin.interface.base\0");
        input.extend_from_slice(self.schema_version.to_le_bytes().as_ref());
        input.extend_from_slice(self.subject.package.as_bytes());
        input.push(0);
        input.extend_from_slice(self.subject.version.as_bytes());
        input.push(0);
        input.extend_from_slice(self.subject.source.as_bytes());
        input.push(0);
        input.extend_from_slice(self.subject.instance.as_bytes());
        input.push(0);
        input.extend_from_slice(&self.semantic_surface_fingerprint.0);
        input.extend_from_slice(&self.public_closure_fingerprint.0);
        Fingerprint::from_bytes(&input)
    }

    pub fn from_subject_and_declarations(
        subject: PackageInstanceId,
        declarations: Vec<InterfaceDeclaration>,
    ) -> Self {
        Self::with_targets_and_realizations(
            subject,
            declarations,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    fn from_subject_and_declarations_base(
        subject: PackageInstanceId,
        mut declarations: Vec<InterfaceDeclaration>,
    ) -> Self {
        declarations.sort_by(|lhs, rhs| lhs.reference.cmp(&rhs.reference));
        let mut previous_key: Option<&DeclarationRef> = None;
        for declaration in &declarations {
            let key = &declaration.reference;
            if previous_key == Some(key) {
                panic!("duplicate declaration key in public interface: {key:?}");
            }
            previous_key = Some(key);
        }
        let surface_input = declarations
            .iter()
            .flat_map(|declaration| {
                let mut bytes = Vec::new();
                declaration.reference.encode_identity(&mut bytes);
                bytes.extend_from_slice(&declaration.fingerprint.0);
                bytes
            })
            .collect::<Vec<_>>();

        let closure_input = declarations
            .iter()
            .fold(Vec::new(), |mut bytes, declaration| {
                declaration.reference.encode_identity(&mut bytes);
                bytes.extend_from_slice(&declaration.fingerprint.0);
                bytes
            });

        Self {
            schema_version: SCHEMA_VERSION,
            subject,
            semantic_surface_fingerprint: Fingerprint::from_domain(
                b"gin.interface.semantic-surface\0",
                SCHEMA_VERSION,
                &surface_input,
            ),
            public_closure_fingerprint: Fingerprint::from_domain(
                b"gin.interface.public-closure\0",
                SCHEMA_VERSION,
                &closure_input,
            ),
            target_profiles: Vec::new(),
            target_realizations: Vec::new(),
            comptime_images: Vec::new(),
            declarations,
        }
    }

    pub fn with_targets_and_realizations(
        subject: PackageInstanceId,
        mut declarations: Vec<InterfaceDeclaration>,
        mut target_profiles: Vec<TargetProfile>,
        target_realizations: Vec<TargetRealization>,
        comptime_images: Vec<ComptimeImage>,
    ) -> Self {
        declarations.sort_by(|lhs, rhs| lhs.reference.cmp(&rhs.reference));
        target_profiles.sort_by_key(|profile| profile.sort_key().to_owned());
        let mut previous_key: Option<&DeclarationRef> = None;
        for declaration in &declarations {
            let key = &declaration.reference;
            if previous_key == Some(key) {
                panic!("duplicate declaration key in public interface: {key:?}");
            }
            previous_key = Some(key);
        }
        let mut previous_profile = None;
        for profile in &target_profiles {
            if previous_profile.is_some_and(|key| key == profile.sort_key()) {
                panic!(
                    "duplicate target profile in public interface: {}",
                    profile.target
                );
            }
            previous_profile = Some(profile.sort_key());
        }
        let mut seen_target = std::collections::HashSet::new();
        let mut target_realizations = target_realizations
            .into_iter()
            .filter(|realization| seen_target.insert(realization.target.clone()))
            .collect::<Vec<_>>();
        target_realizations.sort_by(|lhs, rhs| lhs.target.cmp(&rhs.target));

        let mut seen_image_target = std::collections::HashSet::new();
        let mut comptime_images = comptime_images
            .into_iter()
            .filter(|image| seen_image_target.insert(image.target.clone()))
            .collect::<Vec<_>>();
        comptime_images.sort_by(|lhs, rhs| lhs.target.cmp(&rhs.target));

        let interface = Self::from_subject_and_declarations_base(subject.clone(), declarations);
        let target_realizations = if target_realizations.is_empty() {
            target_profiles
                .iter()
                .map(|profile| TargetRealization {
                    target: profile.target.clone(),
                    semantic_surface_fingerprint: Fingerprint::from_domain(
                        b"gin.interface.target-surface\0",
                        SCHEMA_VERSION,
                        &Self::realization_input(
                            &interface.semantic_surface_fingerprint,
                            &interface.public_closure_fingerprint,
                            std::slice::from_ref(profile),
                        ),
                    ),
                    public_closure_fingerprint: Fingerprint::from_domain(
                        b"gin.interface.target-closure\0",
                        SCHEMA_VERSION,
                        &Self::realization_input(
                            &interface.public_closure_fingerprint,
                            &interface.semantic_surface_fingerprint,
                            std::slice::from_ref(profile),
                        ),
                    ),
                })
                .collect()
        } else {
            let mut target_realizations = target_realizations;
            target_realizations.sort_by(|lhs, rhs| lhs.target.cmp(&rhs.target));
            target_realizations
        };

        Self {
            target_profiles,
            target_realizations,
            comptime_images,
            ..interface
        }
    }

    fn realization_input(
        semantic_surface_fingerprint: &Fingerprint,
        public_closure_fingerprint: &Fingerprint,
        target_profiles: &[TargetProfile],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&semantic_surface_fingerprint.0);
        bytes.extend_from_slice(&public_closure_fingerprint.0);
        for profile in target_profiles {
            bytes.extend_from_slice(profile.target.as_bytes());
        }
        bytes
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let declaration_bytes = self.declarations.iter().map(|declaration| {
            let identity = match &declaration.reference {
                DeclarationRef::Subject {
                    module_path,
                    declaration_path,
                } => module_path.len() + declaration_path.len() + 12,
                DeclarationRef::External {
                    package,
                    module_path,
                    declaration_path,
                } => {
                    package.package.len()
                        + package.version.len()
                        + package.source.len()
                        + package.instance.len()
                        + module_path.len()
                        + declaration_path.len()
                        + 28
                }
            };
            identity + 32
        });
        let section_capacity = 96
            + self.subject.package.len()
            + self.subject.version.len()
            + self.subject.source.len()
            + self.subject.instance.len()
            + declaration_bytes.sum::<usize>()
            + self
                .target_profiles
                .iter()
                .map(|profile| profile.target.len() + 5)
                .sum::<usize>()
            + self
                .target_realizations
                .iter()
                .map(|realization| realization.target.len() + 69)
                .sum::<usize>()
            + self
                .comptime_images
                .iter()
                .map(|image| image.target.len() + 37)
                .sum::<usize>();
        let mut section = Vec::with_capacity(section_capacity);
        section.extend_from_slice(b"S1");
        write_uleb(self.schema_version, &mut section);
        write_string(&self.subject.package, &mut section);
        write_string(&self.subject.version, &mut section);
        write_string(&self.subject.source, &mut section);
        write_string(&self.subject.instance, &mut section);
        section.extend_from_slice(&self.semantic_surface_fingerprint.0);
        section.extend_from_slice(&self.public_closure_fingerprint.0);
        write_uleb(
            u32::try_from(self.declarations.len()).unwrap_or(u32::MAX),
            &mut section,
        );

        for declaration in &self.declarations {
            match &declaration.reference {
                DeclarationRef::Subject {
                    module_path,
                    declaration_path,
                } => {
                    section.push(0);
                    write_string(module_path, &mut section);
                    write_string(declaration_path, &mut section);
                }
                DeclarationRef::External {
                    package,
                    module_path,
                    declaration_path,
                } => {
                    section.push(1);
                    write_string(&package.package, &mut section);
                    write_string(&package.version, &mut section);
                    write_string(&package.source, &mut section);
                    write_string(&package.instance, &mut section);
                    write_string(module_path, &mut section);
                    write_string(declaration_path, &mut section);
                }
            }
            section.extend_from_slice(&declaration.fingerprint.0);
        }
        write_uleb(
            u32::try_from(self.target_profiles.len()).unwrap_or(u32::MAX),
            &mut section,
        );
        for profile in &self.target_profiles {
            write_string(&profile.target, &mut section);
        }
        write_uleb(
            u32::try_from(self.target_realizations.len()).unwrap_or(u32::MAX),
            &mut section,
        );
        for realization in &self.target_realizations {
            write_string(&realization.target, &mut section);
            section.extend_from_slice(&realization.semantic_surface_fingerprint.0);
            section.extend_from_slice(&realization.public_closure_fingerprint.0);
        }
        write_uleb(
            u32::try_from(self.comptime_images.len()).unwrap_or(u32::MAX),
            &mut section,
        );
        for image in &self.comptime_images {
            write_string(&image.target, &mut section);
            section.extend_from_slice(&image.image_fingerprint.0);
        }
        let mut out = Vec::with_capacity(MAGIC.len() + 4 + section.len());
        out.extend_from_slice(MAGIC);
        write_u32(
            u32::try_from(section.len()).unwrap_or_else(|_| panic!("section size overflow")),
            &mut out,
        );
        out.extend_from_slice(&section);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, InterfaceDecodeError> {
        let mut bytes = bytes;
        let magic_len = MAGIC.len();
        let (magic, rest) = bytes
            .split_at_checked(magic_len)
            .ok_or(InterfaceDecodeError::ShortInput)?;
        if magic != MAGIC {
            return Err(InterfaceDecodeError::CorruptHeader);
        }
        bytes = rest;

        let (section_dir_size, rest) = read_u32(bytes)?;
        bytes = rest;
        let section_len =
            usize::try_from(section_dir_size).map_err(|_| InterfaceDecodeError::CorruptHeader)?;
        let (section, tail) = bytes
            .split_at_checked(section_len)
            .ok_or(InterfaceDecodeError::CorruptHeader)?;
        bytes = section;

        let (sig, rest) = bytes
            .split_at_checked(2)
            .ok_or(InterfaceDecodeError::CorruptHeader)?;
        if sig != b"S1" {
            return Err(InterfaceDecodeError::CorruptHeader);
        }
        bytes = rest;

        let (schema_version, rest) = read_uleb(bytes)?;
        if !matches!(schema_version, 1 | SCHEMA_VERSION) {
            return Err(InterfaceDecodeError::UnsupportedSchema(schema_version));
        }
        bytes = rest;

        let (package, rest) = read_string(bytes)?;
        bytes = rest;
        let (version, rest) = read_string(bytes)?;
        bytes = rest;
        let (source, rest) = read_string(bytes)?;
        bytes = rest;
        let (instance, bytes_after_instance) = read_string(bytes)?;
        bytes = bytes_after_instance;

        let (semantic_surface_fingerprint, rest) = read_fp(bytes)?;
        bytes = rest;
        let (public_closure_fingerprint, rest) = read_fp(bytes)?;
        bytes = rest;

        let (declaration_count, rest) = read_uleb(bytes)?;
        bytes = rest;

        let declaration_count =
            usize::try_from(declaration_count).map_err(|_| InterfaceDecodeError::CorruptHeader)?;
        let mut declarations = Vec::with_capacity(declaration_count);
        let mut seen_keys = std::collections::HashSet::new();
        for _ in 0..declaration_count {
            if bytes.is_empty() {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            let kind = bytes[0];
            bytes = &bytes[1..];
            let reference = match kind {
                0 => {
                    let (module_path, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (declaration_path, rest) = read_string(bytes)?;
                    bytes = rest;
                    DeclarationRef::Subject {
                        module_path,
                        declaration_path,
                    }
                }
                1 => {
                    let (package, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (version, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (source, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (instance, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (module_path, rest) = read_string(bytes)?;
                    bytes = rest;
                    let (declaration_path, rest) = read_string(bytes)?;
                    bytes = rest;
                    DeclarationRef::External {
                        package: PackageInstanceId {
                            package,
                            version,
                            source,
                            instance,
                        },
                        module_path,
                        declaration_path,
                    }
                }
                _ => return Err(InterfaceDecodeError::CorruptHeader),
            };
            let (fingerprint, rest) = read_fp(bytes)?;
            bytes = rest;

            if !seen_keys.insert(reference.clone()) {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            declarations.push(InterfaceDeclaration {
                reference,
                fingerprint,
            });
        }
        let (target_profile_count, rest) = if schema_version >= 2 {
            read_uleb(bytes)?
        } else {
            (0, bytes)
        };
        bytes = rest;

        let target_profile_count = usize::try_from(target_profile_count)
            .map_err(|_| InterfaceDecodeError::CorruptHeader)?;
        let mut target_profiles = Vec::with_capacity(target_profile_count);
        let mut seen_target_profiles = std::collections::HashSet::new();
        for _ in 0..target_profile_count {
            let (target, rest) = read_string(bytes)?;
            bytes = rest;
            if !seen_target_profiles.insert(target.clone()) {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            target_profiles.push(TargetProfile { target });
        }

        let (target_realization_count, rest) = if schema_version >= 2 {
            read_uleb(bytes)?
        } else {
            (0, bytes)
        };
        bytes = rest;
        let target_realization_count = usize::try_from(target_realization_count)
            .map_err(|_| InterfaceDecodeError::CorruptHeader)?;
        let mut target_realizations = Vec::with_capacity(target_realization_count);
        let mut seen_realizations = std::collections::HashSet::new();
        for _ in 0..target_realization_count {
            let (target, rest) = read_string(bytes)?;
            bytes = rest;
            let (semantic_surface_fingerprint, rest) = read_fp(bytes)?;
            bytes = rest;
            let (public_closure_fingerprint, rest) = read_fp(bytes)?;
            bytes = rest;
            if !seen_realizations.insert(target.clone()) {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            target_realizations.push(TargetRealization {
                target,
                semantic_surface_fingerprint,
                public_closure_fingerprint,
            });
        }

        let (comptime_image_count, rest) = if schema_version >= 2 {
            read_uleb(bytes)?
        } else {
            (0, bytes)
        };
        bytes = rest;
        let comptime_image_count = usize::try_from(comptime_image_count)
            .map_err(|_| InterfaceDecodeError::CorruptHeader)?;
        let mut comptime_images = Vec::with_capacity(comptime_image_count);
        let mut seen_images = std::collections::HashSet::new();
        for _ in 0..comptime_image_count {
            let (target, rest) = read_string(bytes)?;
            bytes = rest;
            let (image_fingerprint, rest) = read_fp(bytes)?;
            bytes = rest;
            if !seen_images.insert(target.clone()) {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            comptime_images.push(ComptimeImage {
                target,
                image_fingerprint,
            });
        }

        if !bytes.is_empty() {
            return Err(InterfaceDecodeError::CorruptHeader);
        }
        if !tail.is_empty() {
            return Err(InterfaceDecodeError::CorruptHeader);
        }

        Ok(Self {
            schema_version,
            subject: PackageInstanceId {
                package,
                version,
                source,
                instance,
            },
            semantic_surface_fingerprint,
            public_closure_fingerprint,
            declarations,
            target_profiles,
            target_realizations,
            comptime_images,
        })
    }
}

fn write_u32(value: u32, out: &mut Vec<u8>) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u32(bytes: &[u8]) -> Result<(u32, &[u8]), InterfaceDecodeError> {
    if bytes.len() < 4 {
        return Err(InterfaceDecodeError::CorruptHeader);
    }
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[..4]);
    Ok((u32::from_le_bytes(raw), &bytes[4..]))
}

fn write_uleb(mut value: u32, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_uleb(bytes: &[u8]) -> Result<(u32, &[u8]), InterfaceDecodeError> {
    let mut value = 0u32;
    let mut shift = 0u32;
    let mut index = 0usize;
    let mut seen_payload = false;
    loop {
        if index >= bytes.len() {
            return Err(InterfaceDecodeError::CorruptHeader);
        }
        let byte = bytes[index];
        index += 1;
        let payload = u32::from(byte & 0x7f);
        seen_payload = seen_payload || payload != 0;
        value |= payload << shift;
        if byte & 0x80 == 0 {
            if shift > 0 && !seen_payload {
                return Err(InterfaceDecodeError::CorruptHeader);
            }
            return Ok((value, &bytes[index..]));
        }
        shift += 7;
        if shift >= 35 {
            return Err(InterfaceDecodeError::CorruptHeader);
        }
    }
}

fn write_string(value: &str, out: &mut Vec<u8>) {
    write_uleb(u32::try_from(value.len()).unwrap_or(u32::MAX), out);
    out.extend_from_slice(value.as_bytes());
}

fn read_string(bytes: &[u8]) -> Result<(String, &[u8]), InterfaceDecodeError> {
    let (len, rest) = read_uleb(bytes)?;
    let len = usize::try_from(len).map_err(|_| InterfaceDecodeError::CorruptHeader)?;
    if rest.len() < len {
        return Err(InterfaceDecodeError::CorruptHeader);
    }
    let (head, tail) = rest.split_at(len);
    let value =
        String::from_utf8(head.to_vec()).map_err(|_| InterfaceDecodeError::CorruptHeader)?;
    Ok((value, tail))
}

fn read_fp(bytes: &[u8]) -> Result<(Fingerprint, &[u8]), InterfaceDecodeError> {
    let (raw, rest) = bytes
        .split_at_checked(32)
        .ok_or(InterfaceDecodeError::CorruptHeader)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(raw);
    Ok((Fingerprint(out), rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_codec_round_trip() {
        let subject = PackageInstanceId {
            package: "acme/kit".to_string(),
            version: "1.2.3".to_string(),
            source: "local".to_string(),
            instance: "1".to_string(),
        };
        let interface = PublicInterface::from_subject_and_declarations(
            subject,
            vec![
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Foo".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"foo"),
                },
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Bar".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"bar"),
                },
            ],
        );

        let encoded = interface.canonical_bytes();
        let decoded = PublicInterface::decode(&encoded).expect("decode");
        assert_eq!(decoded, interface);
    }

    #[test]
    fn declaration_order_does_not_change_fingerprint() {
        let subject = PackageInstanceId {
            package: "acme/kit".to_string(),
            version: "1.2.3".to_string(),
            source: "local".to_string(),
            instance: "1".to_string(),
        };
        let first = PublicInterface::from_subject_and_declarations(
            subject.clone(),
            vec![
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Foo".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"foo"),
                },
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Bar".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"bar"),
                },
            ],
        );
        let second = PublicInterface::from_subject_and_declarations(
            subject,
            vec![
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Bar".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"bar"),
                },
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "Foo".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"foo"),
                },
            ],
        );
        assert_eq!(first.canonical_bytes(), second.canonical_bytes());
        assert_eq!(
            first.semantic_surface_fingerprint,
            second.semantic_surface_fingerprint
        );
        assert_eq!(
            first.public_closure_fingerprint,
            second.public_closure_fingerprint
        );
    }

    #[test]
    fn decode_rejects_unknown_schema() {
        let subject = PackageInstanceId {
            package: "acme/kit".to_string(),
            version: "1.2.3".to_string(),
            source: "local".to_string(),
            instance: "1".to_string(),
        };
        let mut encoded = PublicInterface::from_subject_and_declarations(
            subject,
            vec![InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "pkg".to_string(),
                    declaration_path: "Foo".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"foo"),
            }],
        )
        .canonical_bytes();

        let sig_pos = encoded
            .windows(2)
            .position(|window| window == b"S1")
            .expect("signature");
        let schema_pos = sig_pos + 2;
        encoded[schema_pos] = 3;
        let decoded = PublicInterface::decode(&encoded);
        assert!(matches!(
            decoded,
            Err(InterfaceDecodeError::UnsupportedSchema(_))
        ));
    }

    #[test]
    fn schema2_round_trip_with_target_realizations_and_images() {
        let subject = PackageInstanceId {
            package: "acme/kit".to_string(),
            version: "1.2.3".to_string(),
            source: "local".to_string(),
            instance: "1".to_string(),
        };
        let interface = PublicInterface::with_targets_and_realizations(
            subject,
            vec![InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "pkg".to_string(),
                    declaration_path: "Foo".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"foo"),
            }],
            vec![TargetProfile {
                target: "x86_64-unknown-linux".to_string(),
            }],
            vec![],
            vec![ComptimeImage {
                target: "x86_64-unknown-linux".to_string(),
                image_fingerprint: Fingerprint::from_bytes(b"image"),
            }],
        );

        let encoded = interface.canonical_bytes();
        let decoded = PublicInterface::decode(&encoded).expect("decode");
        assert_eq!(decoded.target_profiles.len(), 1);
        assert_eq!(decoded.target_profiles[0].target, "x86_64-unknown-linux");
        assert_eq!(decoded.target_realizations.len(), 1);
        assert_eq!(decoded.comptime_images.len(), 1);
        assert_eq!(decoded, interface);
    }

    #[test]
    fn decode_rejects_nonminimal_uleb() {
        let mut encoded = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1".to_string(),
                source: "local".to_string(),
                instance: "1".to_string(),
            },
            vec![],
        )
        .canonical_bytes();
        // Inject a non-minimal uleb for a later field by replacing the schema with 0x80 0x00.
        let sig_pos = encoded
            .windows(2)
            .position(|window| window == b"S1")
            .expect("signature");
        let schema_pos = sig_pos + 2;
        encoded[schema_pos] = 0x80;
        encoded.insert(schema_pos + 1, 0x00);
        assert!(PublicInterface::decode(&encoded).is_err());
    }

    #[test]
    fn decode_rejects_truncated_header_data() {
        let mut encoded = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1".to_string(),
                source: "local".to_string(),
                instance: "1".to_string(),
            },
            vec![],
        )
        .canonical_bytes();
        encoded.truncate(encoded.len() - 1);
        assert!(matches!(
            PublicInterface::decode(&encoded),
            Err(InterfaceDecodeError::CorruptHeader)
        ));
    }

    #[test]
    fn decode_rejects_mismatched_section_length() {
        let mut encoded = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1".to_string(),
                source: "local".to_string(),
                instance: "1".to_string(),
            },
            vec![],
        )
        .canonical_bytes();
        let len_offset = MAGIC.len();
        let bad_len = (u32::MAX).to_le_bytes();
        encoded[len_offset..len_offset + 4].copy_from_slice(&bad_len);
        assert!(matches!(
            PublicInterface::decode(&encoded),
            Err(InterfaceDecodeError::CorruptHeader)
        ));
    }

    #[test]
    fn decode_rejects_unknown_declaration_kind() {
        let mut encoded = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1".to_string(),
                source: "local".to_string(),
                instance: "1".to_string(),
            },
            vec![InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "mod".to_string(),
                    declaration_path: "Value".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"fingerprint"),
            }],
        )
        .canonical_bytes();

        let section_len_offset = MAGIC.len();
        let section_len = u32::from_le_bytes(
            encoded[section_len_offset..section_len_offset + 4]
                .try_into()
                .unwrap(),
        );
        let section_start = section_len_offset + 4;
        let section_end = section_start + section_len as usize;
        let section = encoded.as_mut_slice();

        let mut idx = section_start + 2; // Skip "S1".
        let Ok((schema_version, rest)) = read_uleb(&section[idx..section_end]) else {
            panic!("invalid schema uleb");
        };
        let _ = schema_version;
        idx = section_end - rest.len();

        for _ in 0..4 {
            let Ok((len, rest)) = read_uleb(&section[idx..section_end]) else {
                panic!("invalid package field uleb");
            };
            idx = section_end - rest.len();
            idx += usize::try_from(len).unwrap();
        }
        idx += 32; // semantic_surface_fingerprint
        idx += 32; // public_closure_fingerprint

        let Ok((count, rest)) = read_uleb(&section[idx..section_end]) else {
            panic!("invalid declaration count uleb");
        };
        let _ = count;
        idx = section_end - rest.len();
        assert_eq!(count, 1);

        let kind_pos = idx;
        if kind_pos < section_end {
            section[kind_pos] = 0xFF;
        }

        assert!(matches!(
            PublicInterface::decode(&encoded),
            Err(InterfaceDecodeError::CorruptHeader)
        ));
    }

    #[test]
    fn decode_rejects_corrupt_section_size() {
        let mut encoded = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1".to_string(),
                source: "local".to_string(),
                instance: "1".to_string(),
            },
            vec![],
        )
        .canonical_bytes();
        let section_len_offset = MAGIC.len();
        let bad_len = u32::MAX.to_le_bytes();
        encoded[section_len_offset..section_len_offset + 4].copy_from_slice(&bad_len);
        assert!(matches!(
            PublicInterface::decode(&encoded),
            Err(InterfaceDecodeError::CorruptHeader)
        ));
    }

    #[test]
    fn decode_rejects_uleb_shift_overflow() {
        let bytes = vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x00];
        assert_eq!(read_uleb(&bytes), Err(InterfaceDecodeError::CorruptHeader));
    }

    #[test]
    fn external_references_round_trip() {
        let subject = PackageInstanceId {
            package: "subject/pkg".to_string(),
            version: "1.0.0".to_string(),
            source: "local".to_string(),
            instance: "session".to_string(),
        };
        let external_package = PackageInstanceId {
            package: "dep/pkg".to_string(),
            version: "0.4.2".to_string(),
            source: "cache://dep".to_string(),
            instance: "dep-session".to_string(),
        };
        let interface = PublicInterface::from_subject_and_declarations(
            subject,
            vec![InterfaceDeclaration {
                reference: DeclarationRef::External {
                    package: external_package,
                    module_path: "dep::mod".to_string(),
                    declaration_path: "Symbol".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"sig"),
            }],
        );
        let encoded = interface.canonical_bytes();
        let decoded = PublicInterface::decode(&encoded).expect("decode");
        assert_eq!(decoded, interface);
    }

    #[test]
    fn interface_crate_has_no_compiler_layer_dependencies() {
        const MANIFEST: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));
        assert!(MANIFEST.contains("sha2"));
        assert!(!MANIFEST.contains("parser"));
        assert!(!MANIFEST.contains("resolve"));
        assert!(!MANIFEST.contains("typecheck"));
        assert!(!MANIFEST.contains("analysis"));
        assert!(!MANIFEST.contains("codegen"));
        assert!(!MANIFEST.contains("lsp"));
        assert!(!MANIFEST.contains("ginlsp"));
    }

    #[test]
    fn base_fingerprint_tracks_subject_and_closure() {
        let subject = PackageInstanceId {
            package: "pkg".to_string(),
            version: "1.0.0".to_string(),
            source: "local".to_string(),
            instance: "a".to_string(),
        };
        let decls = vec![
            InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "mod".to_string(),
                    declaration_path: "Value".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"value"),
            },
            InterfaceDeclaration {
                reference: DeclarationRef::Subject {
                    module_path: "mod".to_string(),
                    declaration_path: "Other".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"other"),
            },
        ];

        let base = PublicInterface::from_subject_and_declarations(subject.clone(), decls.clone())
            .base_fingerprint();
        let reordered = PublicInterface::from_subject_and_declarations(
            subject,
            decls.into_iter().rev().collect(),
        )
        .base_fingerprint();
        assert_eq!(base, reordered);

        let changed_subject = PublicInterface::from_subject_and_declarations(
            PackageInstanceId {
                package: "pkg".to_string(),
                version: "1.0.1".to_string(),
                source: "local".to_string(),
                instance: "a".to_string(),
            },
            vec![
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "mod".to_string(),
                        declaration_path: "Value".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"value"),
                },
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "mod".to_string(),
                        declaration_path: "Other".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"other"),
                },
            ],
        )
        .base_fingerprint();
        assert_ne!(base, changed_subject);
    }

    #[test]
    fn base_fingerprint_uses_structural_keys() {
        let subject = PackageInstanceId {
            package: "pkg".to_string(),
            version: "1.0.0".to_string(),
            source: "local".to_string(),
            instance: "a".to_string(),
        };
        let one = PublicInterface::from_subject_and_declarations(
            subject.clone(),
            vec![InterfaceDeclaration {
                reference: DeclarationRef::External {
                    package: PackageInstanceId {
                        package: "dep".to_string(),
                        version: "1".to_string(),
                        source: "s".to_string(),
                        instance: "x".to_string(),
                    },
                    module_path: "dep.mod".to_string(),
                    declaration_path: "Value".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"a"),
            }],
        );
        let another = PublicInterface::from_subject_and_declarations(
            subject,
            vec![InterfaceDeclaration {
                reference: DeclarationRef::External {
                    package: PackageInstanceId {
                        package: "dep".to_string(),
                        version: "1".to_string(),
                        source: "s".to_string(),
                        instance: "y".to_string(),
                    },
                    module_path: "dep.mod".to_string(),
                    declaration_path: "Value".to_string(),
                },
                fingerprint: Fingerprint::from_bytes(b"a"),
            }],
        );
        assert_ne!(one.base_fingerprint(), another.base_fingerprint());
    }

    #[test]
    fn declaration_identity_is_injective_across_fields_and_sources() {
        let external = |package: &str, source: &str, module: &str| DeclarationRef::External {
            package: PackageInstanceId {
                package: package.to_string(),
                version: "1".to_string(),
                source: source.to_string(),
                instance: "root".to_string(),
            },
            module_path: module.to_string(),
            declaration_path: "Value".to_string(),
        };
        let delimiter_left = external("dep:a", "registry", "b");
        let delimiter_right = external("dep", "registry", "a:b");
        let source_left = external("dep", "registry:first", "mod");
        let source_right = external("dep", "registry:second", "mod");

        assert_ne!(delimiter_left, delimiter_right);
        assert_ne!(
            delimiter_left.cmp(&delimiter_right),
            std::cmp::Ordering::Equal
        );
        assert_ne!(source_left, source_right);
        assert_ne!(source_left.cmp(&source_right), std::cmp::Ordering::Equal);

        for (left, right) in [
            (delimiter_left, delimiter_right),
            (source_left, source_right),
        ] {
            let mut left_bytes = Vec::new();
            let mut right_bytes = Vec::new();
            left.encode_identity(&mut left_bytes);
            right.encode_identity(&mut right_bytes);
            assert_ne!(left_bytes, right_bytes);
        }
    }

    #[test]
    #[should_panic]
    fn duplicate_subject_declarations_are_rejected_on_encode() {
        let subject = PackageInstanceId {
            package: "pkg".to_string(),
            version: "1".to_string(),
            source: "local".to_string(),
            instance: "1".to_string(),
        };
        let interface = PublicInterface::from_subject_and_declarations(
            subject,
            vec![
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "A".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"first"),
                },
                InterfaceDeclaration {
                    reference: DeclarationRef::Subject {
                        module_path: "pkg".to_string(),
                        declaration_path: "A".to_string(),
                    },
                    fingerprint: Fingerprint::from_bytes(b"second"),
                },
            ],
        );
        interface.canonical_bytes();
    }
}
