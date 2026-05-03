//! Type environment — [`TyEnv`] construction and lookup methods.

use ast::{Bind, BindValue, Expr, FileAst, ParameterKind};
use internment::Intern;
use std::collections::HashMap;

use crate::resolve::{
    is_type_surface, resolve_name_from_files, resolve_type_expr_from_map,
    resolve_type_expr_with_subst,
};
use crate::ty::{Ty, str_record_ty};
use crate::{LocalTypes, TyInfer, TyInferEnv};

/// Type alias for variant map entries: (union_name, discriminant, fields)
pub(crate) type VariantMapEntry = (Intern<String>, usize, Vec<(Intern<String>, Ty)>);

/// Type alias for the variant map: variant_name -> [(union_name, discriminant, fields)]
pub(crate) type VariantMap = HashMap<Intern<String>, Vec<VariantMapEntry>>;

/// Type alias for variant lookup result: (union_name, discriminant, field_slice)
pub(crate) type VariantLookupResult<'a> = (Intern<String>, usize, &'a [(Intern<String>, Ty)]);

/// Resolved parameter info for a top-level function or method, used to check
/// argument compatibility at call sites with type-variable unification.
#[derive(Clone, PartialEq)]
pub struct FnParamInfo {
    /// Positional parameter types in declaration order.
    pub params: Vec<(Intern<String>, Ty)>,
    /// Method-scoped type variables introduced by the receiver, e.g. `x` in
    /// `Range[x].new`. Empty for non-method binds.
    pub typevars: HashMap<Intern<String>, Ty>,
}

/// Type environment built from a `FileAst`. Resolves tag names to `Ty` and infers
/// function parameter / return types.
#[derive(PartialEq)]
pub struct TyEnv {
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub fn_return_types: HashMap<Intern<String>, Ty>,
    /// Reverse map: variant name → [(parent_union_name, discriminant_index, payload_fields)]
    /// A variant may appear in multiple unions if names collide; shape-based disambiguation is TODO.
    pub variant_map: VariantMap,
    /// Per-function parameter info — populated from each `Bind` so that call
    /// sites can run argument-vs-parameter unification (and reject mismatches
    /// like `Range.new(1, "hi")` where `x` cannot be both `Int` and `Str`).
    pub fn_params: HashMap<Intern<String>, FnParamInfo>,
}

impl TyEnv {
    pub fn from_file_ast(ast: &FileAst) -> Self {
        Self::from_multiple_file_asts(std::slice::from_ref(ast))
    }

    pub fn from_multiple_file_asts(files: &[FileAst]) -> Self {
        let mut tag_types = HashMap::new();

        for ast in files {
            for name in ast.tags.keys() {
                let ty = resolve_name_from_files(*name, files, 0);
                tag_types.insert(*name, ty);
            }
        }

        tag_types
            .entry(Intern::<String>::from_ref("Str"))
            .or_insert_with(str_record_ty);

        let mut variant_map: VariantMap = HashMap::new();
        for (union_name, ty) in &tag_types {
            if let Ty::Union { variants, .. } = ty {
                for (i, (variant_name, fields)) in variants.iter().enumerate() {
                    let field_tys: Vec<(Intern<String>, Ty)> =
                        fields.iter().map(|(n, t)| (*n, *t.clone())).collect();
                    variant_map
                        .entry(*variant_name)
                        .or_default()
                        .push((*union_name, i, field_tys));
                }
            }
        }

        let mut fn_return_types = HashMap::new();
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let env = TyInferEnv {
                    tag_types: &tag_types,
                    fn_return_types: &HashMap::new(),
                    locals: &HashMap::new(),
                };
                let ret = bind.infer_ty(&env);
                fn_return_types.insert(*name, ret);
            }
        }
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let env = TyInferEnv {
                    tag_types: &tag_types,
                    fn_return_types: &fn_return_types,
                    locals: &HashMap::new(),
                };
                let ret = bind.infer_ty(&env);
                fn_return_types.insert(*name, ret);
            }
        }

        // Collect per-function parameter info for call-site unification.
        let mut fn_params: HashMap<Intern<String>, FnParamInfo> = HashMap::new();
        for ast in files {
            for (name, bind) in &ast.defs {
                if !bind.attributes().matches_current_platform() {
                    continue;
                }
                let typevars = bind
                    .receiver_type_surface()
                    .map(|sp| crate::resolve::typevars_from_receiver(&sp.0))
                    .unwrap_or_default();
                let params: Vec<(Intern<String>, Ty)> = match bind.params() {
                    None => Vec::new(),
                    Some(ps) => ps
                        .iter()
                        .map(|(n, kind)| {
                            let ty = crate::infer::resolve_parameter_kind_with_subst(
                                *n,
                                kind,
                                &tag_types,
                                &fn_return_types,
                                &typevars,
                            );
                            (*n, ty)
                        })
                        .collect(),
                };
                fn_params.insert(*name, FnParamInfo { params, typevars });
            }
        }

        TyEnv {
            tag_types,
            fn_return_types,
            variant_map,
            fn_params,
        }
    }

    /// Resolve a type-surface [`Expr`] to a `Ty` using this environment's `tag_types`.
    pub fn resolve_type_expr(&self, e: &Expr) -> Ty {
        resolve_type_expr_from_map(e, &self.tag_types)
    }

    /// Resolve a type-surface [`Expr`] only when `e` is a nominal, qualified, or generic type form.
    pub fn resolve_type_surface(&self, e: &Expr) -> Option<Ty> {
        is_type_surface(e).then(|| resolve_type_expr_from_map(e, &self.tag_types))
    }

    /// Resolve a `ParameterKind` to a `Ty`, consulting a method-scoped
    /// type-variable substitution map.
    ///
    /// `subst` lets `start x` and `end x` in
    /// `Range[x].new(start x, end x) Range[x]: ...` both resolve to the same
    /// `Ty::Opaque(x)`. Pass an empty map for non-method binds.
    pub(crate) fn resolve_parameter_kind_with_subst(
        &self,
        name: Intern<String>,
        kind: &ParameterKind,
        subst: &HashMap<Intern<String>, Ty>,
    ) -> Ty {
        match kind {
            ParameterKind::Tagged(sp) => {
                if is_type_surface(&sp.0) {
                    resolve_type_expr_with_subst(&sp.0, &self.tag_types, subst)
                } else {
                    Ty::Opaque(Intern::<String>::from_ref("?"))
                }
            }
            // Bare-id (`Generic`) params resolve as a fresh per-param type
            // variable `Ty::Opaque(name)` so each call argument can bind it
            // independently. Use `name x` to share a type variable across
            // params (see `Range[x].new(start x, end x)`).
            ParameterKind::Generic => Ty::Opaque(name),
            ParameterKind::Default(expr) => {
                let empty: HashMap<Intern<String>, Ty> = HashMap::new();
                expr.infer_ty(&self.infer_env(&empty))
            }
        }
    }

    /// Return the typed parameter list for a function binding.
    /// Preserves insertion order of the `Parameters` map.
    pub fn param_types<'a>(&self, bind: &'a Bind) -> Vec<(&'a Intern<String>, Ty)> {
        match bind.params().as_ref() {
            None => vec![],
            Some(params) => {
                let subst = bind
                    .receiver_type_surface()
                    .map(|sp| crate::resolve::typevars_from_receiver(&sp.0))
                    .unwrap_or_default();
                params
                    .iter()
                    .map(|(name, kind)| {
                        (
                            name,
                            self.resolve_parameter_kind_with_subst(*name, kind, &subst),
                        )
                    })
                    .collect()
            }
        }
    }

    /// Look up the pre-computed return type of a top-level function by name.
    pub fn fn_return_ty(&self, name: &Intern<String>) -> Option<&Ty> {
        self.fn_return_types.get(name)
    }

    /// Look up a declared type by its tag name.
    pub fn lookup_tag(&self, name: Intern<String>) -> Option<&Ty> {
        self.tag_types.get(&name)
    }

    /// Look up which union a variant belongs to, its discriminant index, and payload fields.
    pub fn lookup_variant(&self, name: Intern<String>) -> Option<VariantLookupResult<'_>> {
        let candidates = self.variant_map.get(&name)?;
        candidates
            .first()
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    /// Return all variant names belonging to `union_name`.
    pub fn all_variants_of(&self, union_name: Intern<String>) -> Vec<Intern<String>> {
        self.variant_map
            .iter()
            .filter_map(|(variant_name, entries)| {
                if entries.iter().any(|(u, _, _)| *u == union_name) {
                    Some(*variant_name)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Build the union→variants reverse map for use in flow analysis display.
    pub fn build_union_to_variants(&self) -> HashMap<Intern<String>, Vec<Intern<String>>> {
        let mut map: HashMap<Intern<String>, Vec<Intern<String>>> = HashMap::new();
        for (variant_name, entries) in &self.variant_map {
            for (union_name, _, _) in entries {
                map.entry(*union_name).or_default().push(*variant_name);
            }
        }
        map
    }

    /// Resolve the union type reachable via a dot expression from `name`.
    pub fn resolve_dot_type(&self, ast: &FileAst, name: Intern<String>) -> Option<Ty> {
        if let Some(ty) = self.lookup_tag(name)
            && ty.is_union()
        {
            return Some(ty.clone());
        }
        let type_name = binding_type_annotation(ast, name)?;
        self.lookup_tag(type_name).cloned()
    }

    /// Build a `TyInferEnv` from this `TyEnv` and a local variable set.
    pub fn infer_env<'a>(&'a self, locals: &'a dyn LocalTypes) -> TyInferEnv<'a> {
        TyInferEnv {
            tag_types: &self.tag_types,
            fn_return_types: &self.fn_return_types,
            locals,
        }
    }
}

fn binding_type_annotation(ast: &FileAst, name: Intern<String>) -> Option<Intern<String>> {
    if let Some(bind) = ast.defs().values().find(|b| b.name() == name) {
        return bind.type_annotation.as_ref().map(|(tn, _)| *tn);
    }
    ast.defs().values().find_map(|bind| {
        let BindValue::Body { exprs, .. } = bind.value() else {
            return None;
        };
        exprs.iter().find_map(|expr| {
            let Expr::Bind(b) = &**expr else { return None };
            if b.name() == name {
                b.type_annotation.as_ref().map(|(tn, _)| *tn)
            } else {
                None
            }
        })
    })
}
