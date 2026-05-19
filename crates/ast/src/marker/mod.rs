//! Compiler marker system — structural type capabilities defined in gin_core.
//!
//! Markers are types defined in `modules/gin_core/marker/*.gin` that carry
//! structural inference rules. The compiler recognizes `Copy` by name.
//! All types are linear (must-use) by default; use `Copy` to opt into
//! implicit copying.
//!
//! Users apply markers to their declarations with `and is` / `and is not` clauses:
//!
//! ```gin
//! Transaction has (id Int)
//!             and is not Copy
//! ```

use internment::Intern;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::ty::Ty;

/// How a marker propagates through container types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkerInference {
    /// Property holds if all field types hold it (AND-like). Used by `Copy`.
    AllFields,
    /// Property holds if any field type holds it (OR-like).
    AnyField,
    /// No structural inference — only explicit `and is` / `and is not`.
    Explicit,
}

/// A parsed marker clause on a declaration: `and is [not] MarkerName(args)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MarkerBinding {
    /// The resolved marker type name (e.g. `Copy`).
    pub marker_name: Intern<String>,
    /// `true` for `and is Copy`, `false` for `and is not Copy`.
    pub positive: bool,
    /// Usage-site arguments, if any. Currently unused but reserved.
    pub args: Vec<String>,
}

/// Compiler-known metadata about a marker type, extracted from its gin_core definition.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerDef {
    /// The marker type name.
    pub name: Intern<String>,
    /// How the marker infers through type structure.
    pub inference: MarkerInference,
    /// If true, writing `and is ThisMarker` implies `and is not Copy` automatically.
    pub implies_not_copy: bool,
    /// Explicit positive bindings: type names that opt into this marker.
    pub positive_bindings: HashSet<Intern<String>>,
    /// Explicit negative bindings: type names that opt out of this marker (`not Copy`).
    pub negative_bindings: HashSet<Intern<String>>,
}

/// The marker registry — built once during type resolution.
///
/// Stores marker definitions from gin_core and explicit `and is` / `and is not`
/// bindings from all declarations in the package.
#[derive(Debug, Clone)]
pub struct MarkerRegistry {
    /// Marker definitions keyed by name.
    definitions: HashMap<Intern<String>, MarkerDef>,
    /// Additional type→marker bindings keyed by marker name, then type name.
    bindings: HashMap<Intern<String>, Vec<(Intern<String>, bool)>>,
}

impl MarkerRegistry {
    pub fn new() -> Self {
        Self {
            definitions: HashMap::new(),
            bindings: HashMap::new(),
        }
    }

    /// Register a marker definition from gin_core.
    pub fn register_definition(&mut self, def: MarkerDef) {
        self.definitions.insert(def.name, def);
    }

    /// Register an explicit binding: a type opting into or out of a marker.
    /// Also updates the marker definition's positive/negative binding sets.
    pub fn register_binding(
        &mut self,
        marker_name: Intern<String>,
        type_name: Intern<String>,
        positive: bool,
    ) {
        // Record the binding
        self.bindings
            .entry(marker_name)
            .or_default()
            .push((type_name, positive));

        // Update the definition's binding sets
        if let Some(def) = self.definitions.get_mut(&marker_name) {
            if positive {
                def.positive_bindings.insert(type_name);
            } else {
                def.negative_bindings.insert(type_name);
            }

            // If this marker implies not Copy, opting in automatically registers
            // a negative Copy binding for this type too.
            if def.implies_not_copy && positive {
                self.definitions
                    .entry(Intern::from_ref("Copy"))
                    .and_modify(|copy_def| {
                        copy_def.negative_bindings.insert(type_name);
                    });
            }
        }
    }

    /// Get a marker definition by name.
    pub fn definition(&self, name: &Intern<String>) -> Option<&MarkerDef> {
        self.definitions.get(name)
    }

    /// Check if a type has an explicit positive binding for a marker.
    pub fn has_positive_binding(
        &self,
        type_name: &Intern<String>,
        marker_name: &Intern<String>,
    ) -> bool {
        self.definitions
            .get(marker_name)
            .map(|def| def.positive_bindings.contains(type_name))
            .unwrap_or(false)
    }

    /// Check if a type has an explicit negative binding for a marker.
    pub fn has_negative_binding(
        &self,
        type_name: &Intern<String>,
        marker_name: &Intern<String>,
    ) -> bool {
        self.definitions
            .get(marker_name)
            .map(|def| def.negative_bindings.contains(type_name))
            .unwrap_or(false)
    }

    /// Returns the names of all recognized markers.
    pub fn recognized_markers(&self) -> Vec<Intern<String>> {
        self.definitions.keys().copied().collect()
    }

    /// Returns true if the marker name is recognized by the compiler.
    pub fn is_recognized(&self, name: &Intern<String>) -> bool {
        self.definitions.contains_key(name)
    }
}

impl Default for MarkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the name of a type, for marker registry lookups.
pub fn ty_name(ty: &Ty) -> Option<&Intern<String>> {
    match ty {
        Ty::Record { name, .. } => Some(name),
        Ty::Union { name, .. } => Some(name),
        Ty::ConstUnion { name, .. } => Some(name),
        Ty::Opaque(name) => Some(name),
        _ => None,
    }
}

/// Check whether a type structurally satisfies a marker according to the
/// marker's inference rule (`AllFields`, `AnyField`, or `Explicit`).
///
/// 1. Explicit negative bindings (`and is not`) short-circuit to `false`.
/// 2. Explicit positive bindings (`and is`) short-circuit to `true`.
/// 3. Otherwise, walk the type structure according to the marker's rule.
pub fn structurally_has_marker(
    ty: &Ty,
    marker_name: &Intern<String>,
    registry: &MarkerRegistry,
) -> bool {
    let Some(def) = registry.definition(marker_name) else {
        return false;
    };

    // Check explicit bindings on this type
    if let Some(name) = ty_name(ty) {
        if def.negative_bindings.contains(name) {
            return false;
        }
        if def.positive_bindings.contains(name) {
            return true;
        }
    }

    // Structurally derive based on inference rule
    let rule = def.inference;
    match ty {
        Ty::Record { fields, .. } => match rule {
            MarkerInference::AllFields => fields
                .iter()
                .all(|(_, f)| structurally_has_marker(f, marker_name, registry)),
            MarkerInference::AnyField => fields
                .iter()
                .any(|(_, f)| structurally_has_marker(f, marker_name, registry)),
            MarkerInference::Explicit => false,
        },
        Ty::Union { variants, .. } => match rule {
            MarkerInference::AllFields => variants.iter().all(|(_, fields)| {
                fields
                    .iter()
                    .all(|(_, f)| structurally_has_marker(f, marker_name, registry))
            }),
            MarkerInference::AnyField => variants.iter().any(|(_, fields)| {
                fields
                    .iter()
                    .any(|(_, f)| structurally_has_marker(f, marker_name, registry))
            }),
            MarkerInference::Explicit => false,
        },
        Ty::Tuple(fields) => match rule {
            MarkerInference::AllFields => fields
                .iter()
                .all(|f| structurally_has_marker(f, marker_name, registry)),
            MarkerInference::AnyField => fields
                .iter()
                .any(|f| structurally_has_marker(f, marker_name, registry)),
            MarkerInference::Explicit => false,
        },
        Ty::Array { elem, .. } => structurally_has_marker(elem, marker_name, registry),
        // Primitives, pointers, refs, opaque — never structurally derive
        Ty::Int { .. } | Ty::Float { .. } | Ty::Bool | Ty::Unit => false,
        Ty::Ptr { .. } => false,
        Ty::Ref { .. } => false,
        Ty::Opaque(_) => false,
        Ty::ConstUnion { base, .. } => structurally_has_marker(base, marker_name, registry),
    }
}
