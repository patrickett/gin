use std::collections::HashMap;

use indexmap::IndexMap;
use internment::Intern;

use crate::TypeExpr;
use crate::doc_comment::DocComment;
use crate::expr::Expr;
use crate::expr::Typed;
use crate::parameter::ParamConvention;
use crate::parameter::ParamSlot;
use crate::parameter::Parameters;
use crate::parameter::fmt_type_expr_surface;
use crate::path::ModPath;
use crate::span::SpanId;
use crate::span::Spanned;
use crate::ty::Ty;
use crate::ty_state::TyState;

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

/// Lazily-formatted method name (e.g., "Single(a).method")
pub struct MethodName<'a> {
    receiver: &'a TypeExpr,
    name: Intern<String>,
}

impl std::fmt::Display for MethodName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt_type_expr_surface(self.receiver, f)?;
        write!(f, ".{}", self.name.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub doc_comment: Option<DocComment>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    pub params: Option<Parameters>,
    pub param_slots: IndexMap<Intern<String>, ParamSlot>,
    pub param_conventions: IndexMap<Intern<String>, ParamConvention>,
    pub attributes: BindAttributes,
    pub value: BindValue,
    /// Method receiver — structural [`TypeExpr`].
    pub receiver_type: Option<Box<Spanned<TypeExpr>>>,
    /// Resolved type variables from the receiver, e.g. `Range[x]` → `{x: Int{...}}`.
    /// Populated during type resolution. Empty for non-method binds.
    pub receiver_typevars: HashMap<Intern<String>, TyState>,
    pub return_type_name: Option<Intern<String>>,
    /// Explicit capitalized return type annotation, e.g. `Str` in `foo() Str: expr`.
    /// Structural [`TypeExpr`].
    pub return_tag: Option<Box<Spanned<TypeExpr>>>,
    /// Resolved/progressive return type. Populated during analysis.
    /// Replaces `return_type_name` + `return_tag` + the `fn_return_types` side-table.
    pub return_type: TyState,
    /// Explicit type annotation with value args, e.g. `Maybe(3)` in `val Maybe(3): Some(3)`.
    pub type_annotation: Option<(Intern<String>, Vec<Typed<Expr>>)>,
    /// Qualified path for type annotation, e.g. `Maybe.Some` in `val Maybe.Some(3): ...`
    pub type_annotation_qual: Option<Spanned<ModPath>>,
    /// `true` for `:=` (immutable/const) binds; `false` for `:` (mutable, alloca-backed) binds.
    pub is_const: bool,
}

impl Bind {
    pub fn new(name: Intern<String>, name_span: SpanId, value: BindValue, is_const: bool) -> Self {
        Bind {
            doc_comment: None,
            name,
            name_span,
            params: None,
            param_slots: IndexMap::new(),
            param_conventions: IndexMap::new(),
            attributes: BindAttributes::default(),
            value,
            receiver_type: None,
            receiver_typevars: HashMap::new(),
            return_type_name: None,
            return_tag: None,
            return_type: TyState::Infer,
            type_annotation: None,
            type_annotation_qual: None,
            is_const,
        }
    }

    pub fn with_return_type_name(mut self, name: Option<Intern<String>>) -> Self {
        self.return_type_name = name;
        self
    }

    pub fn return_type_name(&self) -> Option<&Intern<String>> {
        self.return_type_name.as_ref()
    }

    pub fn with_receiver_type(mut self, receiver_type: Option<Box<Spanned<TypeExpr>>>) -> Self {
        self.receiver_type = receiver_type;
        self
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn name(&self) -> Intern<String> {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        self.doc_comment.as_ref()
    }

    pub fn with_attributes(mut self, attrs: BindAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn attributes(&self) -> &BindAttributes {
        &self.attributes
    }

    pub fn value(&self) -> &BindValue {
        &self.value
    }

    pub fn value_mut(&mut self) -> &mut BindValue {
        &mut self.value
    }

    /// Rename the top-level symbol (used when qualifying module definitions).
    pub fn remap_module_symbol(mut self, symbol: Intern<String>) -> Self {
        self.name = symbol;
        self
    }

    pub fn is_method(&self) -> bool {
        self.receiver_type.is_some()
    }

    /// Return the resolved parameter types in declaration order, with their names.
    /// Only populated after [`populate_ast_types`] or analysis has run.
    /// Returns an empty vec if params haven't been resolved yet.
    pub fn resolved_params(&self) -> Vec<(Intern<String>, Ty)> {
        self.param_slots
            .iter()
            .filter_map(|(name, slot)| match &slot.ty {
                crate::TyState::Resolved(ty) => Some((*name, ty.clone())),
                _ => None,
            })
            .collect()
    }

    /// Return the resolved receiver type variables.
    /// Empty if not a method or not yet resolved.
    pub fn resolved_typevars(&self) -> HashMap<Intern<String>, Ty> {
        self.receiver_typevars
            .iter()
            .filter_map(|(k, tv)| match tv {
                crate::TyState::Resolved(ty) => Some((*k, ty.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn receiver_type_surface(&self) -> Option<&Spanned<TypeExpr>> {
        self.receiver_type.as_deref()
    }

    pub fn method_name(&self) -> Option<MethodName<'_>> {
        let sp = self.receiver_type.as_deref()?;
        Some(MethodName {
            receiver: &sp.value,
            name: self.name,
        })
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
        match &self.params {
            None => 0u8.hash(state),
            Some(params) => {
                1u8.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
        self.is_const.hash(state);
        self.receiver_type.hash(state);
        self.return_tag.hash(state);
        self.return_type_name.hash(state);
        self.type_annotation.hash(state);
        self.type_annotation_qual.hash(state);
        self.value.hash(state);
        for (k, v) in &self.param_conventions {
            k.hash(state);
            v.hash(state);
        }
        // Exclude: param_slots, receiver_typevars, return_type (resolved metadata)
    }
}
