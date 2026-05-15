//! Analysis — type resolution, inference, flow analysis, and diagnostics.

use std::collections::{HashMap, HashSet};

use internment::Intern;

use crate::ty::Ty;

pub mod check;

mod const_value;
pub use const_value::{Bound, ConstValue, TypeConstraint};

mod flow;
pub use flow::{FlowAnalysis, FlowContext, ImpossibleCheck, IndexOutOfBounds};

mod flow_analyzer;
pub use flow_analyzer::FlowAnalyzer;

mod resolve;
pub use resolve::{
    is_type_surface, mangled_fn_call_name, resolve_name_from_files, resolve_type_expr_from_map,
    resolve_type_expr_with_subst, substitute_in_ty, typevars_from_receiver,
};

mod infer;
pub use infer::{
    LayeredLocals, LocalTypes, TyInfer, TyInferEnv, resolve_parameter_kind_with_subst,
};

/// Type alias for variant map entries: (union_name, discriminant, fields)
pub type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
pub type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
pub type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);

/// Results of analyzing a `FileAst` — resolved types, flow, diagnostics.
///
/// Produced by [`crate::resolve_types`] (the free function). All fields are
/// guaranteed to be populated (flow is always `Some` after resolution).
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub fn_return_types: HashMap<Intern<String>, Ty>,
    pub variant_map: VariantMap,
    pub flow: FlowAnalysis,
    pub diagnostics: Vec<diagnostic::Diagnostic>,
    /// Tag names that were explicitly resolved from source files (not fallbacks).
    pub explicit_tag_names: HashSet<Intern<String>>,
}

impl Analysis {
    /// Create an empty analysis. Useful as a placeholder before full resolution.
    pub fn new() -> Self {
        Self {
            tag_types: HashMap::new(),
            fn_return_types: HashMap::new(),
            variant_map: HashMap::new(),
            flow: FlowAnalysis::new(),
            diagnostics: Vec::new(),
            explicit_tag_names: HashSet::new(),
        }
    }

    /// Look up the resolved type for a tag by name.
    pub fn tag_type(&self, name: Intern<String>) -> Option<&Ty> {
        self.tag_types.get(&name)
    }

    /// Like [`tag_type`](Self::tag_type) but takes a `&str`.
    pub fn tag_type_str(&self, name: &str) -> Option<&Ty> {
        self.tag_types.get(&Intern::<String>::from_ref(name))
    }

    /// Look up the inferred return type of a top-level function by name.
    pub fn fn_return_ty(&self, name: &Intern<String>) -> Option<&Ty> {
        self.fn_return_types.get(name)
    }

    /// Look up which union a variant belongs to, its discriminant index, and payload fields.
    pub fn lookup_variant(&self, name: Intern<String>) -> Option<crate::VariantLookupResult<'_>> {
        let candidates = self.variant_map.get(&name)?;
        candidates
            .first()
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    /// Return all variant-map entries for a variant name.
    pub fn all_variants_of(&self, name: Intern<String>) -> Option<&Vec<crate::VariantMapEntry>> {
        self.variant_map.get(&name)
    }
}

impl Default for Analysis {
    fn default() -> Self {
        Self::new()
    }
}
