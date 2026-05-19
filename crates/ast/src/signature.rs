use crate::{Bind, Declare, DeclareValue, FileAst, ParameterKind, Parameters, TypeExpr};
use i256::I256;
use internment::Intern;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;

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

    // Hash only public tags (already sorted)
    for name in ast.public_tag_names() {
        let decl = ast.tags().get(&name).expect("tag should exist");
        hash_tag_def(&mut hasher, &name, decl);
    }

    // Hash only public defs (already sorted), optionally excluding main
    for name in ast.public_def_names() {
        if exclude_main && name.as_str() == "main" {
            continue;
        }
        let bind = ast.defs().get(&name).expect("def should exist");
        hash_def_signature(&mut hasher, &name, bind);
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
fn hash_tag_def(hasher: &mut Sha256, name: &Intern<String>, decl: &Declare) {
    let _ = write!(hasher, "TAG:{}", name);
    hash_parameters(hasher, decl.params());

    match decl.value() {
        DeclareValue::Alias(sp) => {
            let _ = write!(hasher, ":ALIAS:");
            hash_type_expr(hasher, &sp.value);
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
                hash_type_expr(hasher, &variant.shape().value);
                let _ = write!(hasher, "|");
            }
        }
        DeclareValue::Set() => {
            let _ = write!(hasher, ":SET");
        }
        DeclareValue::Range(start, end) => {
            let _ = write!(hasher, ":RANGE:{start}..{end}");
        }
        DeclareValue::InRange(start, end) => {
            let _ = write!(hasher, ":INRANGE:{start}..{end}");
        }
    }
    let _ = write!(hasher, ";");
}

/// Hash a def signature: name + parameter names/types. Body is excluded.
fn hash_def_signature(hasher: &mut Sha256, name: &Intern<String>, bind: &Bind) {
    let _ = write!(hasher, "DEF:{}", name.as_str());
    hash_parameters(hasher, bind.params());
    // Intentionally skip params.1 (BindValue) — that's the body.
    if let Some(complexity) = bind.attributes().complexity.as_ref() {
        let _ = write!(hasher, ":COMPLEXITY:{}", complexity.display_label());
    }
    let _ = write!(hasher, ";");
}

/// Hash an optional parameter list.
fn hash_parameters(hasher: &mut Sha256, params: &Option<Parameters>) {
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
        ParameterKind::Tagged(sp) => {
            let _ = write!(hasher, "TAGGED:");
            if let Some(te) = sp.value.as_type_expr() {
                hash_type_expr(hasher, &te);
            }
        }
        ParameterKind::Default(_) => {
            // Default expressions are part of the implementation, not the interface.
            let _ = write!(hasher, "DEFAULT");
        }
    }
}

fn hash_type_expr(hasher: &mut Sha256, e: &TypeExpr) {
    match e {
        TypeExpr::Nominal(name, _) => {
            let _ = write!(hasher, "N:{}", name);
        }
        TypeExpr::Generic { name, params, .. } => {
            let _ = write!(hasher, "G:{}[", name);
            for (param_name, kind) in params.iter() {
                let _ = write!(hasher, "{param_name}:");
                hash_param_kind(hasher, kind);
                let _ = write!(hasher, ",");
            }
            let _ = write!(hasher, "]");
        }
        TypeExpr::Qualified(path) => {
            let _ = write!(hasher, "Q:{}", path.root);
            for seg in &path.segments {
                let _ = write!(hasher, ".{}", seg);
            }
        }
        TypeExpr::Literal(..) => {
            let _ = write!(hasher, "LIT");
        }
        TypeExpr::Pointer(_) => {
            let _ = write!(hasher, "PTR");
        }
        TypeExpr::Ref { mutable, .. } => {
            if *mutable {
                let _ = write!(hasher, "MUT");
            } else {
                let _ = write!(hasher, "REF");
            }
        }
        TypeExpr::Unit => {
            let _ = write!(hasher, "UNIT");
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

/// Serializable representation of a complexity expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComplexityExprSig {
    Var(String),
    Product(Vec<String>),
    Sum(Vec<String>),
}

/// Serializable representation of a complexity variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComplexitySig {
    Constant,
    Logarithmic(ComplexityExprSig),
    Linear(ComplexityExprSig),
    LogLinear(ComplexityExprSig),
    Quadratic(ComplexityExprSig),
    Cubic(ComplexityExprSig),
    Exponential(ComplexityExprSig),
    Factorial(ComplexityExprSig),
}

impl From<&crate::ComplexityExpr> for ComplexityExprSig {
    fn from(expr: &crate::ComplexityExpr) -> Self {
        match expr {
            crate::ComplexityExpr::Var(v) => ComplexityExprSig::Var(v.as_str().to_string()),
            crate::ComplexityExpr::Product(vars) => {
                ComplexityExprSig::Product(vars.iter().map(|v| v.as_str().to_string()).collect())
            }
            crate::ComplexityExpr::Sum(vars) => {
                ComplexityExprSig::Sum(vars.iter().map(|v| v.as_str().to_string()).collect())
            }
        }
    }
}

impl From<&crate::Complexity> for ComplexitySig {
    fn from(c: &crate::Complexity) -> Self {
        match c {
            crate::Complexity::Constant => ComplexitySig::Constant,
            crate::Complexity::Logarithmic(expr) => ComplexitySig::Logarithmic(expr.into()),
            crate::Complexity::Linear(expr) => ComplexitySig::Linear(expr.into()),
            crate::Complexity::LogLinear(expr) => ComplexitySig::LogLinear(expr.into()),
            crate::Complexity::Quadratic(expr) => ComplexitySig::Quadratic(expr.into()),
            crate::Complexity::Cubic(expr) => ComplexitySig::Cubic(expr.into()),
            crate::Complexity::Exponential(expr) => ComplexitySig::Exponential(expr.into()),
            crate::Complexity::Factorial(expr) => ComplexitySig::Factorial(expr.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefSignature {
    /// Sorted list of (param_name, kind) pairs
    pub params: Vec<(String, ParamKindSig)>,
    /// Time complexity annotation, if present
    pub complexity: Option<ComplexitySig>,
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
#[serde(into = "TagShapeSigRepr", from = "TagShapeSigRepr")]
pub enum TagShapeSig {
    Alias(TagSig),
    Record(Vec<(String, ParamKindSig)>),
    Union(Vec<TagSig>),
    Set,
    Range(I256, I256),
    InRange(I256, I256),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum TagShapeSigRepr {
    Alias(TagSig),
    Record(Vec<(String, ParamKindSig)>),
    Union(Vec<TagSig>),
    Set,
    Range(i128, i128),
    InRange(i128, i128),
}

impl From<TagShapeSig> for TagShapeSigRepr {
    fn from(sig: TagShapeSig) -> Self {
        match sig {
            TagShapeSig::Alias(t) => Self::Alias(t),
            TagShapeSig::Record(v) => Self::Record(v),
            TagShapeSig::Union(v) => Self::Union(v),
            TagShapeSig::Set => Self::Set,
            TagShapeSig::Range(a, b) => Self::Range(a.as_i128(), b.as_i128()),
            TagShapeSig::InRange(a, b) => Self::InRange(a.as_i128(), b.as_i128()),
        }
    }
}

impl From<TagShapeSigRepr> for TagShapeSig {
    fn from(repr: TagShapeSigRepr) -> Self {
        match repr {
            TagShapeSigRepr::Alias(t) => Self::Alias(t),
            TagShapeSigRepr::Record(v) => Self::Record(v),
            TagShapeSigRepr::Union(v) => Self::Union(v),
            TagShapeSigRepr::Set => Self::Set,
            TagShapeSigRepr::Range(a, b) => Self::Range(I256::from(a), I256::from(b)),
            TagShapeSigRepr::InRange(a, b) => Self::InRange(I256::from(a), I256::from(b)),
        }
    }
}

/// Extract a serializable `InterfaceSignature` from a parsed AST.
pub fn extract_interface_signature(ast: &FileAst) -> InterfaceSignature {
    let mut defs = BTreeMap::new();
    let mut tags = BTreeMap::new();

    for name in ast.public_tag_names() {
        let decl = ast.tags().get(&name).expect("tag should exist");
        tags.insert(
            name.to_string(),
            TagSignature {
                params: extract_params(decl.params()),
                shape: extract_tag_shape(decl.value()),
            },
        );
    }

    for name in ast.public_def_names() {
        let bind = ast.defs().get(&name).expect("def should exist");
        defs.insert(
            name.to_string(),
            DefSignature {
                params: extract_params(bind.params()),
                complexity: bind
                    .attributes()
                    .complexity
                    .as_ref()
                    .map(ComplexitySig::from),
            },
        );
    }

    InterfaceSignature { defs, tags }
}

fn extract_params(params: &Option<Parameters>) -> Vec<(String, ParamKindSig)> {
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
        ParameterKind::Tagged(sp) => {
            let sig = sp
                .value
                .as_type_expr()
                .map(|te| extract_type_expr_sig(&te))
                .unwrap_or(TagSig::Nominal(String::new()));
            ParamKindSig::Tagged(sig)
        }
        ParameterKind::Default(_) => ParamKindSig::Default,
    }
}

fn extract_type_expr_sig(e: &TypeExpr) -> TagSig {
    match e {
        TypeExpr::Nominal(name, _) => TagSig::Nominal(name.to_string()),
        TypeExpr::Generic { name, params, .. } => {
            let mut pairs: Vec<_> = params
                .iter()
                .map(|(n, k)| (n.to_string(), extract_param_kind(k)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            TagSig::Generic(name.to_string(), pairs)
        }
        TypeExpr::Qualified(path) => {
            let mut parts = vec![path.root.to_string()];
            for seg in &path.segments {
                parts.push(seg.to_string());
            }
            TagSig::Qualified(parts)
        }
        TypeExpr::Literal(..) => TagSig::Nominal(String::new()),
        TypeExpr::Pointer(_) | TypeExpr::Ref { .. } | TypeExpr::Unit => {
            TagSig::Nominal(String::new())
        }
    }
}

fn extract_tag_shape(value: &DeclareValue) -> TagShapeSig {
    match value {
        DeclareValue::Alias(sp) => TagShapeSig::Alias(extract_type_expr_sig(&sp.value)),
        DeclareValue::Record(params) => {
            let mut pairs: Vec<_> = params
                .iter()
                .map(|(n, k)| (n.to_string(), extract_param_kind(k)))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            TagShapeSig::Record(pairs)
        }
        DeclareValue::Union { variants } => TagShapeSig::Union(
            variants
                .iter()
                .map(|v| extract_type_expr_sig(&v.shape().value))
                .collect(),
        ),
        DeclareValue::Set() => TagShapeSig::Set,
        DeclareValue::Range(start, end) => TagShapeSig::Range(*start, *end),
        DeclareValue::InRange(start, end) => TagShapeSig::InRange(*start, *end),
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
