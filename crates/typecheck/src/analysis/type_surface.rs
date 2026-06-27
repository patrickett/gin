//! Raw type resolution — translating AST type declarations into resolved [`Ty`] values.
//!
//! These functions operate on the raw [`DeclareValue`] tree without a resolved
//! `HashMap`. Resolution unfolds aliases, computes union variants and const-unions,
//! and produces the canonical [`Ty`] that other passes consume.

use crate::ty::Ty;
use ast::{DeclareValue, FnCall, InRangeBounds, ParamKind, ParameterKind, Parameters, TypeExpr};
use internment::Intern;
use std::collections::HashMap;

/// Bundles the context needed to resolve [`TypeExpr`] to [`Ty`].
///
/// Pass this through the resolution tree instead of threading individual parameters.
/// The `subst`, `tag_params`, and `tag_decls` fields are optional — most callers
/// only need `tag_types`.
pub struct TypeEnv<'a> {
    pub tag_types: &'a HashMap<Intern<String>, Ty>,
    pub subst: Option<&'a HashMap<Intern<String>, Ty>>,
    pub tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
    pub tag_decls: Option<&'a ast::TagMap>,
}

impl<'a> TypeEnv<'a> {
    pub fn new(tag_types: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types,
            subst: None,
            tag_params: None,
            tag_decls: None,
        }
    }

    pub fn with_subst(&self, subst: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: Some(subst),
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
        }
    }

    pub fn with_tag_params(&self, tag_params: &'a HashMap<Intern<String>, Parameters>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: self.subst,
            tag_params: Some(tag_params),
            tag_decls: self.tag_decls,
        }
    }

    pub fn with_opt_tag_params(
        &self,
        tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
    ) -> Self {
        match tag_params {
            Some(p) => self.with_tag_params(p),
            None => TypeEnv {
                tag_types: self.tag_types,
                subst: self.subst,
                tag_params: None,
                tag_decls: self.tag_decls,
            },
        }
    }

    pub fn with_tag_decls(&self, tag_decls: &'a ast::TagMap) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: self.subst,
            tag_params: self.tag_params,
            tag_decls: Some(tag_decls),
        }
    }

    /// Resolve a type surface expression to a [`Ty`].
    pub fn resolve(&self, e: &TypeExpr) -> Ty {
        match e {
            TypeExpr::Nominal(name, _) => {
                if let Some(subst) = self.subst
                    && let Some(t) = subst.get(name)
                {
                    return t.clone();
                }
                // Unresolved lowercase names are type variables.
                if let Some(c) = name.as_str().chars().next()
                    && c.is_ascii_lowercase()
                {
                    return Ty::Opaque(*name);
                }
                self.tag_types
                    .get(name)
                    .cloned()
                    .unwrap_or(Ty::Opaque(*name))
            }
            TypeExpr::Generic { name, params, .. } => {
                if let Some(subst) = self.subst
                    && params
                        .iter()
                        .all(|(_, k)| matches!(k, ParameterKind::Generic))
                    && params.len() == 1
                    && let Some((var, _)) = params.first()
                    && let Some(ty) = subst.get(var)
                {
                    return ty.clone();
                }
                let empty = HashMap::new();
                let local_subst = self.build_subst_for_generic(params, name);
                let merged_subst = merge_subst(self.subst.unwrap_or(&empty), &local_subst);
                let with_subst = self.with_subst(&merged_subst);
                if !local_subst.is_empty()
                    && let Some(tag_decls) = self.tag_decls
                    && let Some(ast::DeclareValue::Interface(members)) =
                        tag_decls.get(name).map(|d| &d.value)
                {
                    let fields: Vec<(Intern<String>, Box<Ty>)> = members
                        .iter()
                        .map(|m| {
                            let ty = m
                                .return_ty
                                .as_ref()
                                .map(|rt| with_subst.resolve(&rt.value))
                                .unwrap_or(Ty::Unit);
                            (m.name, Box::new(ty))
                        })
                        .collect();
                    return Ty::Record {
                        name: *name,
                        fields,
                    };
                }
                if params.is_empty()
                    && let Some(tag_decls) = self.tag_decls
                    && let Some(ast::DeclareValue::Alias(sp)) =
                        tag_decls.get(name).map(|d| &d.value)
                {
                    return with_subst.resolve(&sp.value);
                }
                let base = self
                    .tag_types
                    .get(name)
                    .cloned()
                    .unwrap_or(Ty::Opaque(*name));
                if merged_subst.is_empty() {
                    base
                } else {
                    base.substitute(&merged_subst)
                }
            }
            TypeExpr::Qualified(path) => self
                .tag_types
                .get(&path.root)
                .cloned()
                .unwrap_or(Ty::Opaque(path.root)),
            TypeExpr::Literal(..) => Ty::Opaque(Intern::new(String::new())),
            TypeExpr::Pointer(inner) => Ty::Ptr {
                inner: Box::new(self.resolve(&inner.value)),
            },
            TypeExpr::Ref { inner, mutable } => Ty::Ref {
                inner: Box::new(self.resolve(&inner.value)),
                mutable: *mutable,
            },
            TypeExpr::Unit => Ty::Unit,
            TypeExpr::InRange { bounds, .. } => resolve_in_range_bounds(bounds, self.tag_types),
            TypeExpr::ListEmpty | TypeExpr::ListCons { .. } => Ty::Unit,
            TypeExpr::Tuple(elems) => {
                Ty::Tuple(elems.iter().map(|e| self.resolve(&e.value)).collect())
            }
        }
    }

    fn build_subst_for_generic(
        &self,
        use_site_params: &[(Intern<String>, ParameterKind)],
        tag_name: &Intern<String>,
    ) -> HashMap<Intern<String>, Ty> {
        let mut out = HashMap::new();

        if let Some(tag_params) = self.tag_params
            && let Some(decl_params) = tag_params.get(tag_name)
        {
            let decl_entries: Vec<_> = decl_params.iter().collect();
            for (i, (name, kind)) in use_site_params.iter().enumerate() {
                let resolved = match kind {
                    ParameterKind::Tagged(sp) if sp.value.is_type_surface() => {
                        self.resolve(&sp.value)
                    }
                    ParameterKind::Generic => Ty::Opaque(*name),
                    _ => continue,
                };
                if let Some((decl_name, _)) = decl_entries.get(i) {
                    out.insert(**decl_name, resolved);
                }
            }
            for (decl_name, decl_kind) in decl_entries.iter().skip(use_site_params.len()) {
                if let ParameterKind::Default(expr) = decl_kind
                    && let Some(te) = expr.value.as_type_expr()
                    && te.is_type_surface()
                {
                    let default_ty = self.resolve(&te);
                    out.insert(**decl_name, default_ty);
                }
            }
        } else {
            for (name, kind) in use_site_params {
                let resolved = match kind {
                    ParameterKind::Tagged(sp) if sp.value.is_type_surface() => {
                        self.resolve(&sp.value)
                    }
                    ParameterKind::Generic => Ty::Opaque(*name),
                    _ => continue,
                };
                out.insert(*name, resolved);
            }
        }
        out
    }
}

// ---- Internal helpers ----

fn merge_subst(
    outer: &HashMap<Intern<String>, Ty>,
    local: &HashMap<Intern<String>, Ty>,
) -> HashMap<Intern<String>, Ty> {
    let mut merged = outer.clone();
    merged.extend(local.iter().map(|(k, v)| (*k, v.clone())));
    merged
}

fn resolve_in_range_bounds(bounds: &InRangeBounds, tag_types: &HashMap<Intern<String>, Ty>) -> Ty {
    match bounds {
        InRangeBounds::Literal(min, max) => Ty::bounded_int(*min, *max),
        InRangeBounds::Tag(name) => {
            if let Some(ty) = tag_types.get(name) {
                if ty.is_bounded_int() {
                    return ty.clone();
                }
                if let Some(b) = ty.scalar_bounds_from_range_value() {
                    return Ty::Int {
                        width: ty_int_width(ty).unwrap_or(64),
                        signed: b.min < 0,
                        value: None,
                        min: Some(b.min),
                        max: Some(b.max),
                    };
                }
            }
            Ty::Opaque(*name)
        }
    }
}

fn ty_int_width(ty: &Ty) -> Option<u8> {
    match ty {
        Ty::Record { fields, .. } => {
            fields
                .iter()
                .find(|(n, _)| n.as_str() == "start")
                .and_then(|(_, t)| match t.as_ref() {
                    Ty::Int { width, .. } => Some(*width),
                    _ => None,
                })
        }
        Ty::Int { width, .. } => Some(*width),
        _ => None,
    }
}

/// Flattens a qualified path into a single symbol name for codegen (e.g. `io.print`).
pub fn mangled_fn_call_name(call: &FnCall) -> Intern<String> {
    if call.path.segments.is_empty() {
        call.path.root
    } else {
        let mut joined = call.path.root.as_str().to_string();
        for seg in &call.path.segments {
            joined.push('.');
            joined.push_str(seg.as_str());
        }
        Intern::<String>::new(joined)
    }
}

pub fn resolve_name_from_files(
    name: Intern<String>,
    files: &[ast::FileAst],
    _recursion_depth: usize,
) -> Ty {
    let mut raw: HashMap<Intern<String>, &DeclareValue> = HashMap::new();
    for ast in files {
        for (k, v) in ast.tags.iter() {
            raw.insert(*k, &v.value);
        }
    }
    if let Some(dv) = raw.get(&name) {
        match dv {
            DeclareValue::Alias(spanned) => {
                let empty = HashMap::new();
                let env = TypeEnv::new(&empty);
                return env.resolve(&spanned.value);
            }
            DeclareValue::Interface(members) => {
                let empty = HashMap::new();
                let env = TypeEnv::new(&empty);
                let fields: Vec<(Intern<String>, Box<Ty>)> = members
                    .iter()
                    .map(|m| {
                        let ty = m
                            .return_ty
                            .as_ref()
                            .map(|rt| env.resolve(&rt.value))
                            .unwrap_or(Ty::Unit);
                        (m.name, Box::new(ty))
                    })
                    .collect();
                return Ty::Record { name, fields };
            }
            DeclareValue::Union { .. } => {
                return Ty::Union {
                    name,
                    variants: Vec::new(),
                    literal_values: None,
                };
            }
            _ => {}
        }
    }
    Ty::Opaque(name)
}

/// Check that the provided type application arguments match the declaration's expected kinds.
pub fn check_type_application(
    declared_params: &[(Intern<String>, ParamKind)],
    provided_params: &Parameters,
) -> Result<(), String> {
    if provided_params.len() != declared_params.len() {
        return Err(format!(
            "expected {} type arguments, found {}",
            declared_params.len(),
            provided_params.len()
        ));
    }

    for (i, ((_decl_name, decl_kind), (_, prov_kind))) in declared_params
        .iter()
        .zip(provided_params.iter())
        .enumerate()
    {
        match (decl_kind, prov_kind) {
            (ParamKind::Type, ParameterKind::Tagged(_))
            | (ParamKind::Type, ParameterKind::Generic) => {}
            (ParamKind::Type, _) => {
                return Err(format!(
                    "expected type argument for `{}`, found const value",
                    declared_params[i].0.as_str()
                ));
            }
            (ParamKind::Value(_), ParameterKind::Tagged(_))
            | (ParamKind::Value(_), ParameterKind::Generic) => {}
            (ParamKind::Value(_), _) => {
                return Err(format!(
                    "expected const argument for `{}`, found type",
                    declared_params[i].0.as_str()
                ));
            }
        }
    }
    Ok(())
}
