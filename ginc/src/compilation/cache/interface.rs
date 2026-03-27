use crate::ast::{Declare, DeclareValue, FileAst, ParameterKind, Tag, Variant};
use crate::intern::IStr;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;

/// Compute a SHA-256 hex digest of raw source text.
///
/// This replaces `FileAst::compute_content_hash()` (which uses `DefaultHasher`
/// and is not stable across processes) with a cryptographic hash suitable for
/// on-disk caching.
///
/// This function now performs semantic hashing by lexing the source and hashing
/// only the non-comment tokens. This means that adding/removing comments does not
/// invalidate the cache, significantly improving incremental compilation performance.
pub fn compute_content_hash(source: &str) -> String {
    use crate::lexer::GinLexer;

    let mut hasher = Sha256::new();

    // Lex the source and hash only non-comment tokens
    // The lexer's Iterator impl already filters out comments
    let lexer = GinLexer::new(source);
    for (token, _span) in lexer {
        // Hash the token discriminant and payload
        // This is stable across runs because we use the token's Debug representation
        // which includes both the variant and any associated data
        let token_str = format!("{:?}", token);
        hasher.update(token_str.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// Compute a SHA-256 hex digest of a `FileAst`'s public API surface.
///
/// The interface hash captures tag and def *signatures* but excludes:
/// - Function bodies (`BindValue::Expr` / `BindValue::Body`)
/// - Default parameter value expressions (hashed as `"DEFAULT"`)
/// - Doc comments
///
/// Changing a function body without touching its signature produces the
/// same interface hash, so dependents skip recompilation.
pub fn compute_interface_hash(ast: &FileAst) -> String {
    compute_lib_interface_hash(ast, false)
}

/// Compute interface hash, optionally excluding the 'main' def.
///
/// This is used for library interface hashing where the 'main' function
/// from the binary entry point should be excluded from the library's public API.
pub fn compute_lib_interface_hash(ast: &FileAst, exclude_main: bool) -> String {
    let mut hasher = Sha256::new();

    let tags = ast.tags();
    let defs = ast.defs();
    // Hash only public tags sorted by name for determinism
    let mut tag_names: Vec<&IStr> = tags
        .keys()
        .filter(|n| !ast.private_tags().contains(n))
        .collect();
    tag_names.sort();

    for name in tag_names {
        let decl = &tags[name];
        hash_tag_def(&mut hasher, name, decl);
    }

    // Hash only public defs sorted by name for determinism
    let mut def_names: Vec<&IStr> = defs
        .keys()
        .filter(|n| !ast.private_defs().contains(n))
        .filter(|n| !(exclude_main && n.as_str() == "main"))
        .collect();
    def_names.sort();

    for name in def_names {
        let bind = &defs[name];
        hash_def_signature(&mut hasher, name, bind);
    }

    format!("{:x}", hasher.finalize())
}

/// Compute aggregated interface hash from multiple .gin files.
///
/// This combines the interface hashes of all files in a project to create
/// a single hash representing the library's complete public API.
/// The entry file (if specified) will have its 'main' function excluded.
pub fn compute_aggregated_interface_hash(
    files: &[(std::path::PathBuf, FileAst)],
    entry_file: Option<&std::path::Path>,
) -> String {
    let mut hasher = Sha256::new();

    for (path, ast) in files {
        let is_entry_file = entry_file
            .map(|entry| {
                path.file_name()
                    .and_then(|f| f.to_str())
                    .map(|f| entry.file_name().and_then(|e| e.to_str()) == Some(f))
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        let hash = compute_lib_interface_hash(ast, is_entry_file);
        hasher.update(hash.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// Hash a tag definition: name + shape + parameter signatures.
fn hash_tag_def(hasher: &mut Sha256, name: &IStr, decl: &Declare) {
    let _ = write!(hasher, "TAG:{}", name);
    hash_parameters(hasher, decl.params());

    match decl.value() {
        DeclareValue::Alias(tag) => {
            let _ = write!(hasher, ":ALIAS:");
            hash_tag(hasher, tag);
        }
        DeclareValue::Record(parameters) => {
            let _ = write!(hasher, ":RECORD:");
            for (param_name, kind) in parameters {
                let _ = write!(hasher, "{param_name}:");
                hash_param_kind(hasher, kind);
                let _ = write!(hasher, ",");
            }
        }
        DeclareValue::Union { variants } => {
            let _ = write!(hasher, ":UNION:");
            for variant in variants {
                hash_tag(hasher, extract_variant_tag(variant));
                let _ = write!(hasher, "|");
            }
        }
        DeclareValue::Set() => {
            let _ = write!(hasher, ":SET");
        }
        DeclareValue::Range(range) => {
            let _ = write!(hasher, ":RANGE:{}..{}", range.start, range.end);
        }
        DeclareValue::InRange(range) => {
            let _ = write!(hasher, ":INRANGE:{}..{}", range.start, range.end);
        }
    }
    let _ = write!(hasher, ";");
}

/// Hash a def signature: name + parameter names/types. Body is excluded.
fn hash_def_signature(
    hasher: &mut Sha256,
    name: &IStr,
    bind: &crate::ast::Bind,
) {
    let _ = write!(hasher, "DEF:{}", name.as_str());
    hash_parameters(hasher, bind.params());
    // Intentionally skip params.1 (BindValue) — that's the body.
    let _ = write!(hasher, ";");
}

/// Hash an optional parameter list.
fn hash_parameters(
    hasher: &mut Sha256,
    params: &Option<crate::ast::Parameters>,
) {
    match params {
        Some(parameters) => {
            let _ = write!(hasher, "(");
            for (param_name, kind) in parameters {
                let _ = write!(hasher, "{param_name}:");
                hash_param_kind(hasher, kind);
                let _ = write!(hasher, ",");
            }
            let _ = write!(hasher, ")");
        }
        None => {
            let _ = write!(hasher, "()");
        }
    }
}

/// Hash a parameter kind — tags are hashed structurally, defaults are opaque.
fn hash_param_kind(hasher: &mut Sha256, kind: &ParameterKind) {
    match kind {
        ParameterKind::Generic => {
            let _ = write!(hasher, "GENERIC");
        }
        ParameterKind::Tagged(tag) => {
            let _ = write!(hasher, "TAGGED:");
            hash_tag(hasher, tag);
        }
        ParameterKind::Default(_) => {
            // Default expressions are part of the implementation, not the interface.
            let _ = write!(hasher, "DEFAULT");
        }
    }
}

// ── Interface Signature: extractable, serializable, diffable ──────────

/// The level of semver bump required.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SemverBump {
    None,
    Patch,
    Minor,
    Major,
}

/// A serializable snapshot of a module's public API surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceSignature {
    pub defs: BTreeMap<String, DefSignature>,
    pub tags: BTreeMap<String, TagSignature>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefSignature {
    /// Sorted list of (param_name, kind) pairs
    pub params: Vec<(String, ParamKindSig)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSignature {
    pub params: Vec<(String, ParamKindSig)>,
    pub shape: TagShapeSig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamKindSig {
    Generic,
    Tagged(TagSig),
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagSig {
    Nominal(String),
    Generic(String, Vec<(String, ParamKindSig)>),
    Qualified(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagShapeSig {
    Alias(TagSig),
    Record(Vec<(String, ParamKindSig)>),
    Union(Vec<TagSig>),
    Set,
    Range(i64, i64),
    InRange(i64, i64),
}

/// Extract a serializable `InterfaceSignature` from a parsed AST.
pub fn extract_interface_signature(ast: &FileAst) -> InterfaceSignature {
    let mut defs = BTreeMap::new();
    let mut tags = BTreeMap::new();

    for (name, decl) in ast.tags() {
        if ast.private_tags().contains(name) {
            continue;
        }
        tags.insert(
            name.to_string(),
            TagSignature {
                params: extract_params(decl.params()),
                shape: extract_tag_shape(decl.value()),
            },
        );
    }

    for (name, bind) in ast.defs() {
        if ast.private_defs().contains(name) {
            continue;
        }
        defs.insert(
            name.to_string(),
            DefSignature {
                params: extract_params(bind.params()),
            },
        );
    }

    InterfaceSignature { defs, tags }
}

fn extract_params(
    params: &Option<crate::ast::Parameters>,
) -> Vec<(String, ParamKindSig)> {
    match params {
        Some(parameters) => {
            let mut pairs: Vec<_> = parameters
                .iter()
                .map(|(name, kind)| (name.to_string(), extract_param_kind(kind)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs
        }
        None => Vec::new(),
    }
}

fn extract_param_kind(kind: &ParameterKind) -> ParamKindSig {
    match kind {
        ParameterKind::Generic => ParamKindSig::Generic,
        ParameterKind::Tagged(tag) => ParamKindSig::Tagged(extract_tag_sig(tag)),
        ParameterKind::Default(_) => ParamKindSig::Default,
    }
}

fn extract_tag_sig(tag: &Tag) -> TagSig {
    match tag {
        Tag::Nominal(name, _) => TagSig::Nominal(name.to_string()),
        Tag::Generic(name, parameters, _) => {
            let mut pairs: Vec<_> = parameters
                .iter()
                .map(|(n, k)| (n.to_string(), extract_param_kind(k)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            TagSig::Generic(name.to_string(), pairs)
        }
        Tag::Qualified(path) => {
            let mut parts = vec![path.root.to_string()];
            for seg in &path.segments {
                parts.push(seg.to_string());
            }
            TagSig::Qualified(parts)
        }
    }
}

fn extract_tag_shape(value: &DeclareValue) -> TagShapeSig {
    match value {
        DeclareValue::Alias(tag) => TagShapeSig::Alias(extract_tag_sig(tag)),
        DeclareValue::Record(params) => {
            let mut pairs: Vec<_> = params
                .iter()
                .map(|(n, k)| (n.to_string(), extract_param_kind(k)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            TagShapeSig::Record(pairs)
        }
        DeclareValue::Union { variants } => {
            TagShapeSig::Union(variants.iter().map(|v| extract_tag_sig(extract_variant_tag(v))).collect())
        }
        DeclareValue::Set() => TagShapeSig::Set,
        DeclareValue::Range(r) => TagShapeSig::Range(r.start, r.end),
        DeclareValue::InRange(r) => TagShapeSig::InRange(r.start, r.end),
    }
}

/// Helper to extract the tag from a Variant (for hashing and signature extraction)
fn extract_variant_tag(variant: &Variant) -> &Tag {
    match variant {
        Variant::External(tag) => tag,
        Variant::Local { tag, .. } => tag,
    }
}

/// Compare two interface signatures and determine the required semver bump.
///
/// - Removed or changed defs/tags → `Major` (breaking)
/// - Added defs/tags → `Minor` (additive)
/// - No interface change → `None` (caller can upgrade to `Patch` if content changed)
pub fn diff_interfaces(old: &InterfaceSignature, new: &InterfaceSignature) -> SemverBump {
    let mut bump = SemverBump::None;

    // Check defs
    for (name, old_sig) in &old.defs {
        match new.defs.get(name) {
            None => return SemverBump::Major, // removed
            Some(new_sig) if new_sig != old_sig => return SemverBump::Major, // changed
            _ => {}
        }
    }
    for name in new.defs.keys() {
        if !old.defs.contains_key(name) {
            bump = bump.max(SemverBump::Minor); // added
        }
    }

    // Check tags
    for (name, old_sig) in &old.tags {
        match new.tags.get(name) {
            None => return SemverBump::Major,
            Some(new_sig) if new_sig != old_sig => return SemverBump::Major,
            _ => {}
        }
    }
    for name in new.tags.keys() {
        if !old.tags.contains_key(name) {
            bump = bump.max(SemverBump::Minor);
        }
    }

    bump
}

/// Apply a semver bump to a version string.
///
/// For `0.x.y`: breaking → bump minor, additive → bump patch.
/// For `>=1.x.y`: breaking → bump major, additive → bump minor, patch → bump patch.
pub fn apply_bump(version: &str, bump: SemverBump) -> Option<String> {
    let ver = semver::Version::parse(version).ok()?;
    let new_ver = match bump {
        SemverBump::None => return Some(version.to_string()),
        SemverBump::Patch => semver::Version::new(ver.major, ver.minor, ver.patch + 1),
        SemverBump::Minor => {
            if ver.major == 0 {
                // Pre-1.0: minor bump for additive
                semver::Version::new(0, ver.minor, ver.patch + 1)
            } else {
                semver::Version::new(ver.major, ver.minor + 1, 0)
            }
        }
        SemverBump::Major => {
            if ver.major == 0 {
                // Pre-1.0: breaking = bump minor
                semver::Version::new(0, ver.minor + 1, 0)
            } else {
                semver::Version::new(ver.major + 1, 0, 0)
            }
        }
    };
    Some(new_ver.to_string())
}

/// Hash a Tag type structurally.
fn hash_tag(hasher: &mut Sha256, tag: &Tag) {
    match tag {
        Tag::Nominal(name, _) => {
            let _ = write!(hasher, "N:{}", name);
        }
        Tag::Generic(name, parameters, _) => {
            let _ = write!(hasher, "G:{}(", name);
            for (param_name, kind) in parameters {
                let _ = write!(hasher, "{param_name}:");
                hash_param_kind(hasher, kind);
                let _ = write!(hasher, ",");
            }
            let _ = write!(hasher, ")");
        }
        Tag::Qualified(path) => {
            let _ = write!(hasher, "Q:{}", path.root);
            for seg in &path.segments {
                let _ = write!(hasher, ".{}", seg);
            }
        }
    }
}
