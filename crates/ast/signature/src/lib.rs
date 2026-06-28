//! Interface signature types and hashing for the Gin compiler.
//!
//! These types capture a module's public API surface for semver analysis.
//! They are only used by the `ginc` compiler for version bump computation,
//! not by the core compilation pipeline.

use ast::{
    Bind, Declare, DeclareValue, FileAst, InRangeBounds, ParameterKind, Parameters, TypeExpr,
};
use ast::{Complexity, ComplexityExpr};
use i256::I256;
use internment::Intern;

#[cfg(feature = "serialization")]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;

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

        let hash = ast.lib_interface_hash(is_entry_file);
        hasher.update(hash.as_bytes());
    }

    format!("{:x}", hasher.finalize())
}

/// Hash a tag definition: name + shape + parameter signatures.
fn hash_tag_def(hasher: &mut Sha256, name: &Intern<String>, decl: &Declare) {
    let _ = write!(hasher, "TAG:{}", name);
    hash_parameters(hasher, decl.params.as_ref());

    match &decl.value {
        DeclareValue::Alias(sp) => {
            let _ = write!(hasher, ":ALIAS:");
            hash_type_expr(hasher, &sp.value);
        }
        DeclareValue::Interface(members) => {
            let _ = write!(hasher, ":INTERFACE:");
            for m in members {
                let _ = write!(hasher, "{}:", m.name.as_str());
                hash_parameters(hasher, Some(&m.params));
                for (_k, conv) in &m.conventions {
                    let _ = write!(hasher, "{:?}", conv);
                }
                if let Some(rt) = &m.return_ty {
                    let _ = write!(hasher, ":RET:");
                    hash_type_expr(hasher, &rt.value);
                }
                if let Some(et) = &m.error_ty {
                    let _ = write!(hasher, ":ERR:");
                    hash_type_expr(hasher, &et.value);
                }
                let _ = write!(hasher, ";");
            }
        }
        DeclareValue::Union { variants } => {
            let _ = write!(hasher, ":UNION:");
            for variant in variants {
                hash_type_expr(hasher, &variant.shape().value);
                let _ = write!(hasher, "|");
            }
        }
        DeclareValue::When(_) => {
            let _ = write!(hasher, ":WHEN:");
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

    // Hash provided traits from `Type.Trait(...)` implementation declarations.
    for pt in &decl.provided_traits {
        let _ = write!(hasher, ":PROVIDES:{}:", pt.trait_name.as_str());
        for (field_name, _) in &pt.fields {
            let _ = write!(hasher, "{field_name}:EXPR,");
        }
    }

    let _ = write!(hasher, ";");
}

/// Hash a def signature: name + parameter names/types. Body is excluded.
fn hash_def_signature(hasher: &mut Sha256, name: &Intern<String>, bind: &Bind) {
    let _ = write!(hasher, "DEF:{}", name.as_str());
    hash_parameters(hasher, bind.params.as_ref());
    // Intentionally skip params.1 (BindValue) — that's the body.
    if let Some(complexity) = bind.attributes.complexity.as_ref() {
        let _ = write!(hasher, ":COMPLEXITY:{}", complexity.display_label());
    }
    let _ = write!(hasher, ";");
}

/// Hash an optional parameter list.
fn hash_parameters(hasher: &mut Sha256, params: Option<&Parameters>) {
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
            hash_type_expr(hasher, &sp.value);
        }
        ParameterKind::ValueParam { ty } => {
            let _ = write!(hasher, "VALUE:");
            hash_type_expr(hasher, &ty.value);
        }
        ParameterKind::Default(_) => {
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
        TypeExpr::InRange { bounds, .. } => {
            let _ = write!(hasher, "IR:");
            match bounds {
                InRangeBounds::Literal(a, b) => {
                    let _ = write!(hasher, "{a}..{b}");
                }
                InRangeBounds::Tag(t) => {
                    let _ = write!(hasher, "{}", t);
                }
            }
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
        TypeExpr::ListEmpty => {
            let _ = write!(hasher, "LIST_EMPTY");
        }
        TypeExpr::ListCons { head, tail } => {
            let _ = write!(hasher, "LIST_CONS:");
            hash_type_expr(hasher, &head.value);
            hash_type_expr(hasher, &tail.value);
        }
        TypeExpr::Tuple(elems) => {
            let _ = write!(hasher, "TUPLE:");
            for e in elems {
                hash_type_expr(hasher, &e.value);
            }
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
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSignature {
    pub defs: BTreeMap<String, DefSignature>,
    pub tags: BTreeMap<String, TagSignature>,
}

/// Serializable representation of a complexity expression.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComplexityExprSig {
    Var(String),
    Product(Vec<String>),
    Sum(Vec<String>),
}

/// Serializable representation of a complexity variant.
#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl From<&ComplexityExpr> for ComplexityExprSig {
    fn from(expr: &ComplexityExpr) -> Self {
        match expr {
            ComplexityExpr::Var(v) => ComplexityExprSig::Var(v.as_str().to_string()),
            ComplexityExpr::Product(vars) => {
                ComplexityExprSig::Product(vars.iter().map(|v| v.as_str().to_string()).collect())
            }
            ComplexityExpr::Sum(vars) => {
                ComplexityExprSig::Sum(vars.iter().map(|v| v.as_str().to_string()).collect())
            }
        }
    }
}

impl From<&Complexity> for ComplexitySig {
    fn from(c: &Complexity) -> Self {
        match c {
            Complexity::Constant => ComplexitySig::Constant,
            Complexity::Logarithmic(expr) => ComplexitySig::Logarithmic(expr.into()),
            Complexity::Linear(expr) => ComplexitySig::Linear(expr.into()),
            Complexity::LogLinear(expr) => ComplexitySig::LogLinear(expr.into()),
            Complexity::Quadratic(expr) => ComplexitySig::Quadratic(expr.into()),
            Complexity::Cubic(expr) => ComplexitySig::Cubic(expr.into()),
            Complexity::Exponential(expr) => ComplexitySig::Exponential(expr.into()),
            Complexity::Factorial(expr) => ComplexitySig::Factorial(expr.into()),
        }
    }
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefSignature {
    /// Sorted list of (param_name, kind) pairs
    pub params: Vec<(String, ParamKindSig)>,
    /// Time complexity annotation, if present
    pub complexity: Option<ComplexitySig>,
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSignature {
    pub params: Vec<(String, ParamKindSig)>,
    pub shape: TagShapeSig,
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamKindSig {
    Generic,
    Tagged(TagSig),
    Default,
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagSig {
    Nominal(String),
    Generic(String, Vec<(String, ParamKindSig)>),
    Qualified(Vec<String>),
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serialization",
    serde(into = "TagShapeSigRepr", from = "TagShapeSigRepr")
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagShapeSig {
    Alias(TagSig),
    Record(Vec<(String, ParamKindSig)>),
    Union(Vec<TagSig>),
    Set,
    Range(I256, I256),
    InRange(I256, I256),
    Interface(Vec<(String, String)>),
}

#[cfg_attr(feature = "serialization", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TagShapeSigRepr {
    Alias(TagSig),
    Record(Vec<(String, ParamKindSig)>),
    Union(Vec<TagSig>),
    Set,
    Range(i128, i128),
    InRange(i128, i128),
    Interface(Vec<(String, String)>),
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
            TagShapeSig::Interface(members) => Self::Interface(members),
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
            TagShapeSigRepr::Interface(members) => Self::Interface(members),
        }
    }
}

fn extract_params(params: Option<&Parameters>) -> Vec<(String, ParamKindSig)> {
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
            let sig = extract_type_expr_sig(&sp.value);
            ParamKindSig::Tagged(sig)
        }
        ParameterKind::ValueParam { ty } => {
            let sig = extract_type_expr_sig(&ty.value);
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
        TypeExpr::InRange { .. }
        | TypeExpr::Pointer(_)
        | TypeExpr::Ref { .. }
        | TypeExpr::Unit
        | TypeExpr::ListEmpty
        | TypeExpr::ListCons { .. }
        | TypeExpr::Tuple(_) => TagSig::Nominal(String::new()),
    }
}

fn extract_tag_shape(value: &DeclareValue) -> TagShapeSig {
    match value {
        DeclareValue::Alias(sp) => TagShapeSig::Alias(extract_type_expr_sig(&sp.value)),
        DeclareValue::Interface(members) => {
            let sigs: Vec<ast::declare::InterfaceMember> = members.clone();
            TagShapeSig::Interface(
                sigs.into_iter()
                    .map(|m| (m.name.to_string(), "method".to_string()))
                    .collect(),
            )
        }
        DeclareValue::Union { variants } => TagShapeSig::Union(
            variants
                .iter()
                .map(|v| extract_type_expr_sig(&v.shape().value))
                .collect(),
        ),
        DeclareValue::When(_) => TagShapeSig::Alias(TagSig::Nominal("when".to_string())),
        DeclareValue::Set() => TagShapeSig::Set,
        DeclareValue::Range(start, end) => TagShapeSig::Range(*start, *end),
        DeclareValue::InRange(start, end) => TagShapeSig::InRange(*start, *end),
    }
}

/// Extension trait providing interface hash and signature methods on [`FileAst`].
pub trait FileAstSignatureExt {
    /// Compute a SHA-256 hex digest of this AST's public API surface.
    fn interface_hash(&self) -> String;

    /// Compute interface hash, optionally excluding the 'main' def.
    fn lib_interface_hash(&self, exclude_main: bool) -> String;

    /// Extract a serializable [`InterfaceSignature`] from this AST.
    fn interface_signature(&self) -> InterfaceSignature;
}

impl FileAstSignatureExt for FileAst {
    fn interface_hash(&self) -> String {
        self.lib_interface_hash(false)
    }

    fn lib_interface_hash(&self, exclude_main: bool) -> String {
        let mut hasher = Sha256::new();

        // Hash only public tags (already sorted)
        for name in self.public_tag_names() {
            let decl = self.tags.get(&name).expect("tag should exist");
            hash_tag_def(&mut hasher, &name, decl);
        }

        // Hash only public defs (already sorted), optionally excluding main
        for name in self.public_def_names() {
            if exclude_main && name.as_str() == "main" {
                continue;
            }
            let bind = self.defs.get(&name).expect("def should exist");
            hash_def_signature(&mut hasher, &name, bind);
        }

        format!("{:x}", hasher.finalize())
    }

    fn interface_signature(&self) -> InterfaceSignature {
        let mut defs = BTreeMap::new();
        let mut tags = BTreeMap::new();

        for name in self.public_tag_names() {
            let decl = self.tags.get(&name).expect("tag should exist");
            tags.insert(
                name.to_string(),
                TagSignature {
                    params: extract_params(decl.params.as_ref()),
                    shape: extract_tag_shape(&decl.value),
                },
            );
        }

        for name in self.public_def_names() {
            let bind = self.defs.get(&name).expect("def should exist");
            defs.insert(
                name.to_string(),
                DefSignature {
                    params: extract_params(bind.params.as_ref()),
                    complexity: bind.attributes.complexity.as_ref().map(ComplexitySig::from),
                },
            );
        }

        InterfaceSignature { defs, tags }
    }
}

impl InterfaceSignature {
    /// Compare two interface signatures and determine the required semver bump.
    pub fn diff(&self, other: &InterfaceSignature) -> SemverBump {
        let mut bump = SemverBump::None;

        // Check defs
        for (name, old_sig) in &self.defs {
            match other.defs.get(name) {
                None => return SemverBump::Major, // removed
                Some(new_sig) if new_sig != old_sig => return SemverBump::Major, // changed
                _ => {}
            }
        }
        for name in other.defs.keys() {
            if !self.defs.contains_key(name) {
                bump = bump.max(SemverBump::Minor); // added
            }
        }

        // Check tags
        for (name, old_sig) in &self.tags {
            match other.tags.get(name) {
                None => return SemverBump::Major,
                Some(new_sig) if new_sig != old_sig => return SemverBump::Major,
                _ => {}
            }
        }
        for name in other.tags.keys() {
            if !self.tags.contains_key(name) {
                bump = bump.max(SemverBump::Minor);
            }
        }

        bump
    }
}

impl SemverBump {
    /// Apply this semver bump to a version string.
    pub fn apply(self, version: &str) -> Option<String> {
        let ver = semver::Version::parse(version).ok()?;
        let new_ver = match self {
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
}
