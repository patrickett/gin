//! Raw type resolution — translating AST type declarations into resolved [`Ty`] values.
//!
//! These functions operate on the raw [`DeclareValue`] tree without a resolved
//! `HashMap`. Resolution unfolds aliases, computes union variants and const-unions,
//! and produces the canonical [`Ty`] that other passes consume.

use crate::subst::DepSubst;
use crate::ty::Ty;
use crate::analysis::{TyInfer, TyInferEnv};
use ast::{
    BinderId, NormalExpr, DeclareValue, DependentArgId, FnCall, InRangeBounds, Literal, ParamKind,
    ParameterKind, Parameters, TyArg,
};
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
    pub dependent_binder: Option<BinderId>,
}

impl<'a> TypeEnv<'a> {
    pub fn new(tag_types: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types,
            subst: None,
            tag_params: None,
            tag_decls: None,
            dependent_binder: None,
        }
    }

    pub fn with_subst(&self, subst: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: Some(subst),
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            dependent_binder: self.dependent_binder,
        }
    }

    pub fn with_tag_params(&self, tag_params: &'a HashMap<Intern<String>, Parameters>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: self.subst,
            tag_params: Some(tag_params),
            tag_decls: self.tag_decls,
            dependent_binder: self.dependent_binder,
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
                dependent_binder: self.dependent_binder,
            },
        }
    }

    pub fn with_tag_decls(&self, tag_decls: &'a ast::TagMap) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: self.subst,
            tag_params: self.tag_params,
            tag_decls: Some(tag_decls),
            dependent_binder: self.dependent_binder,
        }
    }

    pub fn with_dependent_binder(&self, dependent_binder: BinderId) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            subst: self.subst,
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            dependent_binder: Some(dependent_binder),
        }
    }

    /// Resolve a type surface expression to a [`Ty`].
    fn build_dep_subst_for_generic(
        &self,
        use_site_params: &[(Intern<String>, ParameterKind)],
        tag_name: &Intern<String>,
    ) -> DepSubst {
        let mut types = HashMap::new();
        let mut consts = HashMap::new();

        if let Some(tag_params) = self.tag_params
            && let Some(decl_params) = tag_params.get(tag_name)
        {
            // Validate that use-site param kinds match declared param kinds.
            let declared: Vec<(Intern<String>, ParamKind)> = decl_params
                .iter()
                .map(|(n, k)| (*n, param_kind_from_decl_parameter_kind(&k.kind)))
                .collect();
            let _ = check_type_application(&declared, use_site_params);
            let decl_entries: Vec<_> = decl_params.iter().collect();
            for (i, (name, kind)) in use_site_params.iter().enumerate() {
                if let Some((decl_name, _)) = decl_entries.get(i) {
                    match kind {
                        ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp }
                            if is_literal_expr(&sp.value) =>
                        {
                            if let Some(cv) = literal_expr_as_const(&sp.value) {
                                consts.insert(**decl_name, cv);
                            }
                        }
                        ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp }
                            if expr_is_type_surface(&sp.value) =>
                        {
                            let ty = self.resolve_expr(&sp.value);
                            types.insert(**decl_name, ty);
                        }
                        ParameterKind::Generic => {
                            types.insert(**decl_name, Ty::Opaque(*name));
                        }
                        ParameterKind::Default(expr) => {
                            if expr_is_type_surface(&expr.value) {
                            types.insert(**decl_name, self.resolve_expr(&expr.value));
                        } else if let Some(value) = expr_as_size_normal_expr(&expr.value) {
                                consts.insert(**decl_name, value);
                            }
                        }
                        ParameterKind::Inferred { .. }
                        | ParameterKind::Tagged(_)
                        | ParameterKind::ValueParam { .. } => continue,
                    }
                }
            }
            for (ordinal, (decl_name, decl_kind)) in
                decl_entries.iter().enumerate().skip(use_site_params.len())
            {
                match &decl_kind.kind {
                    ParameterKind::Default(expr) => {
                        if expr_is_type_surface(&expr.value) {
                            types.insert(**decl_name, self.resolve_expr(&expr.value));
                        } else if let Some(value) = expr_as_size_normal_expr(&expr.value) {
                            consts.insert(**decl_name, value);
                        }
                    }
                    ParameterKind::Inferred { .. } => {
                        if let Some(binder) = self.dependent_binder {
                            consts.insert(
                                **decl_name,
                                NormalExpr::Inferred(DependentArgId::new(binder, ordinal as u32)),
                            );
                        }
                    }
                    _ => {}
                }
            }
        } else {
            for (name, kind) in use_site_params {
                match kind {
                    ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp }
                        if is_literal_expr(&sp.value) =>
                    {
                        if let Some(cv) = literal_expr_as_const(&sp.value) {
                            consts.insert(*name, cv);
                        }
                    }
                    ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp }
                        if expr_is_type_surface(&sp.value) =>
                    {
                        let ty = self.resolve_expr(&sp.value);
                        types.insert(*name, ty);
                    }
                    ParameterKind::Generic => {
                        types.insert(*name, Ty::Opaque(*name));
                    }
                    ParameterKind::Default(expr) => {
                        if expr_is_type_surface(&expr.value) {
                        types.insert(*name, self.resolve_expr(&expr.value));
                        } else if let Some(value) = expr_as_size_normal_expr(&expr.value) {
                            consts.insert(*name, value);
                        }
                    }
                    ParameterKind::Inferred { .. }
                    | ParameterKind::Tagged(_)
                    | ParameterKind::ValueParam { .. } => continue,
                }
            }
        }
        DepSubst::from_maps(types, consts)
    }

    pub fn resolve(&self, expr: &ast::Expr) -> Ty {
        match expr {
            ast::Expr::AnonymousTag(name) => {
                if let Some(subst) = self.subst
                    && let Some(t) = subst.get(name)
                {
                    return t.clone();
                }
                if name
                    .as_str()
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_lowercase())
                {
                    return Ty::Opaque(*name);
                }
                self.tag_types
                    .get(name)
                    .cloned()
                    .unwrap_or(Ty::Opaque(*name))
            }
            ast::Expr::TagCall(call) => {
                let params: Vec<(Intern<String>, ParameterKind)> = call
                    .args
                    .iter()
                    .enumerate()
                    .map(|(index, arg)| {
                        (
                            Intern::new(format!("_{index}")),
                            ParameterKind::Default(Box::new(arg.clone())),
                        )
                    })
                    .collect();
                self.resolve_generic(&call.name, &params)
            }
            ast::Expr::Ref {
                inner, mutable, ..
            } => Ty::Ref {
                inner: Box::new(self.resolve_expr(&inner.value)),
                mutable: *mutable,
            },
            ast::Expr::TakePtr(inner) => Ty::Ptr {
                inner: Box::new(self.resolve_expr(&inner.value)),
            },
            ast::Expr::FnCall(call) if call.args.is_none() => {
                if call.path.value.segments.is_empty() {
                    self.resolve(&ast::Expr::AnonymousTag(call.path.value.root))
                } else {
                    self.tag_types
                        .get(&call.path.value.root)
                        .cloned()
                        .unwrap_or(Ty::Opaque(call.path.value.root))
                }
            }
            ast::Expr::Lit(_) => Ty::Opaque(Intern::new(String::new())),
            ast::Expr::Range(range) => match (&range.start.value, &range.end.value) {
                (ast::Expr::AnonymousTag(start), ast::Expr::AnonymousTag(end)) if start == end => {
                    resolve_in_range_bounds(&InRangeBounds::Tag(*start), self.tag_types)
                }
                (
                    ast::Expr::Lit(ast::Literal::Number(start)),
                    ast::Expr::Lit(ast::Literal::Number(end)),
                ) => resolve_in_range_bounds(
                    &InRangeBounds::Literal(
                        i256::I256::from_u128(*start as u128),
                        i256::I256::from_u128(*end as u128),
                    ),
                    self.tag_types,
                ),
                (
                    ast::Expr::Lit(ast::Literal::Int(start)),
                    ast::Expr::Lit(ast::Literal::Int(end)),
                ) => resolve_in_range_bounds(
                    &InRangeBounds::Literal(
                        i256::I256::from_u128(*start),
                        i256::I256::from_u128(*end),
                    ),
                    self.tag_types,
                ),
                _ => Ty::Opaque(Intern::from_ref("<expr>")),
            },
            _ => Ty::Opaque(Intern::from_ref("<expr>")),
        }
    }

    pub fn resolve_expr(&self, expr: &ast::Expr) -> Ty {
        self.resolve(expr)
    }

    fn resolve_generic(
        &self,
        name: &Intern<String>,
        params: &[(Intern<String>, ParameterKind)],
    ) -> Ty {
        if name.as_str() == "Array" && params.len() == 2 {
            let (element_name, element_kind) = &params[0];
            let element = match element_kind {
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => self.resolve_expr(&ty.value),
                ParameterKind::Generic => self
                    .subst
                    .and_then(|subst| subst.get(element_name))
                    .cloned()
                    .unwrap_or(Ty::Opaque(*element_name)),
                ParameterKind::Default(expr) => infer_default_type_arg(expr, self),
            };
            let (size_name, size_kind) = &params[1];
            let size = match size_kind {
                ParameterKind::Default(expr) => {
                    expr_as_size_normal_expr(&expr.value).unwrap_or(NormalExpr::Var(*size_name))
                }
                ParameterKind::Tagged(ty)
                | ParameterKind::ValueParam { ty }
                | ParameterKind::Inferred { ty } => {
                    literal_expr_as_const(&ty.value).unwrap_or(NormalExpr::Var(*size_name))
                }
                ParameterKind::Generic => NormalExpr::Var(*size_name),
            };
            return Ty::Array {
                elem: Box::new(element),
                size,
            };
        }

        if let Some(subst) = self.subst
            && params.iter().all(|(_, kind)| matches!(kind, ParameterKind::Generic))
            && params.len() == 1
            && let Some((var, _)) = params.first()
            && let Some(ty) = subst.get(var)
        {
            return ty.clone();
        }
        let local_dep = self.build_dep_subst_for_generic(params, name);
        let empty = HashMap::new();
        let merged_types = merge_subst(self.subst.unwrap_or(&empty), &local_dep.types);
        let with_subst = self.with_subst(&merged_types);
        let any_subst = !local_dep.types.is_empty() || !local_dep.consts.is_empty();
        if any_subst
            && let Some(tag_decls) = self.tag_decls
            && let Some(ast::DeclareValue::Has(members)) = tag_decls.get(name).map(|d| &d.value)
        {
            let fields: Vec<(Intern<String>, Box<Ty>)> = members
                .iter()
                .filter_map(|member| match member {
                    ast::HasMember::Property(property) => {
                        let ty = property
                            .ty
                            .as_ref()
                            .map(|ty| with_subst.resolve_expr(&ty.value))
                            .unwrap_or(Ty::Unit);
                        Some((property.name, Box::new(ty)))
                    }
                    ast::HasMember::Function(_) => None,
                })
                .collect();
            return Ty::Record {
                name: *name,
                fields,
                resolved_params: self.resolved_params(name, params, &local_dep),
            };
        }
        if params.is_empty()
            && let Some(tag_decls) = self.tag_decls
            && let Some(ast::DeclareValue::Alias(sp)) = tag_decls.get(name).map(|d| &d.value)
        {
            return with_subst.resolve_expr(&sp.value);
        }
        let base = self
            .tag_types
            .get(name)
            .cloned()
            .unwrap_or(Ty::Opaque(*name));
        if merged_types.is_empty() && local_dep.consts.is_empty() {
            base
        } else {
            let subst = DepSubst::from_maps(merged_types, local_dep.consts);
            let mut resolved = subst.apply_to_ty(&base);
            if let Ty::Union {
                ref mut resolved_params,
                ..
            } = resolved
            {
                *resolved_params = self.resolved_params(name, params, &subst);
            }
            if let Ty::Record {
                ref mut resolved_params,
                ..
            } = resolved
            {
                *resolved_params = self.resolved_params(name, params, &subst);
            }
            resolved
        }
    }

    fn resolved_params(
        &self,
        tag_name: &Intern<String>,
        use_site_params: &[(Intern<String>, ParameterKind)],
        subst: &DepSubst,
    ) -> Option<Vec<(Intern<String>, TyArg)>> {
        let names: Vec<Intern<String>> = self
            .tag_params
            .and_then(|tag_params| tag_params.get(tag_name))
            .map(|params| params.iter().map(|(name, _)| *name).collect())
            .unwrap_or_else(|| use_site_params.iter().map(|(name, _)| *name).collect());
        let params: Vec<_> = names
            .into_iter()
            .filter_map(|name| {
                subst
                    .types
                    .get(&name)
                    .map(|ty| (name, TyArg::Type(Box::new(ty.clone()))))
                    .or_else(|| {
                        subst
                            .consts
                            .get(&name)
                            .map(|expr| (name, TyArg::Const(expr.clone())))
                    })
            })
            .collect();
        (!params.is_empty()).then_some(params)
    }
}

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
                return env.resolve_expr(&spanned.value);
            }
            DeclareValue::Has(members) => {
                let empty = HashMap::new();
                let env = TypeEnv::new(&empty);
                let fields: Vec<(Intern<String>, Box<Ty>)> = members
                    .iter()
                    .filter_map(|member| match member {
                        ast::HasMember::Property(property) => {
                            let ty = property
                                .ty
                                .as_ref()
                            .map(|ty| env.resolve_expr(&ty.value))
                                .unwrap_or(Ty::Unit);
                            Some((property.name, Box::new(ty)))
                        }
                        ast::HasMember::Function(_) => None,
                    })
                    .collect();
                return Ty::Record {
                    name,
                    fields,
                    resolved_params: None,
                };
            }
            DeclareValue::Union { .. } => {
                return Ty::Union {
                    name,
                    variants: Vec::new(),
                    literal_values: None,
                    resolved_params: None,
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
    provided_params: &[(Intern<String>, ParameterKind)],
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
            | (ParamKind::Type, ParameterKind::Generic)
            | (ParamKind::Type, ParameterKind::Default(_)) => {}
            (ParamKind::Type, ParameterKind::ValueParam { .. })
            | (ParamKind::Type, ParameterKind::Inferred { .. }) => {
                return Err(format!(
                    "expected type argument for `{}`, found const value",
                    declared_params[i].0.as_str()
                ));
            }
            (ParamKind::Value(_), ParameterKind::Tagged(_))
            | (ParamKind::Value(_), ParameterKind::Generic)
            | (ParamKind::Value(_), ParameterKind::ValueParam { .. })
            | (ParamKind::Value(_), ParameterKind::Inferred { .. })
            | (ParamKind::Value(_), ParameterKind::Default(_)) => {}
        }
    }
    Ok(())
}

/// Convert a tag declaration's [`ParameterKind`] to the resolved [`ParamKind`].
/// The inner type for `Value` is a placeholder since only the Type-vs-Value
/// discriminant is used during kind-checking.
fn param_kind_from_decl_parameter_kind(kind: &ParameterKind) -> ParamKind {
    match kind {
        ParameterKind::Generic | ParameterKind::Tagged(_) | ParameterKind::Default(_) => {
            ParamKind::Type
        }
        ParameterKind::ValueParam { ty: _ } | ParameterKind::Inferred { ty: _ } => {
            ParamKind::Value(Box::new(crate::ty::Ty::i64()))
        }
    }
}

fn is_literal_expr(expr: &ast::Expr) -> bool {
    matches!(expr, ast::Expr::Lit(Literal::Int(_) | Literal::Number(_)))
}

fn literal_expr_as_const(expr: &ast::Expr) -> Option<NormalExpr> {
    match expr {
        ast::Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::Lit(Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        _ => None,
    }
}

pub(crate) fn expr_is_type_surface(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::AnonymousTag(_)
        | ast::Expr::TagCall(_)
        | ast::Expr::FnCall(ast::FnCall { args: None, .. }) => true,
        ast::Expr::Ref { inner, .. } | ast::Expr::TakePtr(inner) => {
            expr_is_type_surface(&inner.value)
        }
        ast::Expr::Range(range) => match (&range.start.value, &range.end.value) {
            (ast::Expr::AnonymousTag(start), ast::Expr::AnonymousTag(end)) if start == end => true,
            (
                ast::Expr::Lit(ast::Literal::Number(_)),
                ast::Expr::Lit(ast::Literal::Number(_)),
            )
            | (
                ast::Expr::Lit(ast::Literal::Int(_)),
                ast::Expr::Lit(ast::Literal::Int(_)),
            ) => true,
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn typevars_from_receiver_expr(
    expr: &ast::Expr,
) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    let ast::Expr::TagCall(call) = expr else {
        return out;
    };
    for arg in &call.args {
        if let ast::Expr::AnonymousTag(name) = arg.value
            && name.as_str().chars().next().is_some_and(|c| c.is_ascii_lowercase())
        {
            out.insert(name, Ty::Opaque(name));
        }
    }
    out
}

fn expr_as_size_normal_expr(expr: &ast::Expr) -> Option<NormalExpr> {
    match expr {
        ast::Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::Lit(Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::FnCall(call) if call.args.is_none() => {
            Some(NormalExpr::Var(call.path.value.root))
        }
        _ => expr_as_size_normal_expr_without_calls(expr),
    }
}

fn expr_as_size_normal_expr_without_calls(expr: &ast::Expr) -> Option<NormalExpr> {
    match expr {
        ast::Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::Lit(Literal::Number(n)) => Some(NormalExpr::from(*n as i128)),
        ast::Expr::Bind(bind) => Some(NormalExpr::Var(bind.name)),
        ast::Expr::Binary(binary) => {
            let lhs = expr_as_size_normal_expr_without_calls(&binary.lhs.value)?;
            let rhs = expr_as_size_normal_expr_without_calls(&binary.rhs.value)?;
            Some(match binary.op {
                ast::expr::BinOp::Add => NormalExpr::Add(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Subtract => NormalExpr::Sub(Box::new(lhs), Box::new(rhs)),
                ast::expr::BinOp::Multiply => NormalExpr::Mul(Box::new(lhs), Box::new(rhs)),
                _ => return None,
            })
        }
        _ => None,
    }
}

fn infer_default_type_arg(expr: &ast::Typed<ast::Expr>, env: &TypeEnv<'_>) -> Ty {
    if expr_is_type_surface(&expr.value) {
        return env.resolve_expr(&expr.value);
    }
    let infer_env = TyInferEnv {
        tag_types: env.tag_types,
        fn_return_types: &HashMap::new(),
        locals: &HashMap::new(),
        tag_params: env.tag_params,
    };
    expr.infer_ty(&infer_env)
}
