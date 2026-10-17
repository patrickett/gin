//! Raw type resolution — translating AST type declarations into resolved [`Ty`] values.
//!
//! These functions operate on the raw [`DeclareValue`] tree without a resolved
//! `HashMap`. Resolution unfolds aliases, computes union variants and const-unions,
//! and produces the canonical [`Ty`] that other passes consume.

use crate::normal_expr::{SizeCallName, size_expr_to_normal};
use crate::subst::DepSubst;
use crate::tag_declarations::TagDeclarations;
use crate::transform::when_values::union_type;
use crate::ty::Ty;
use ast::type_decl::TypeNameExt;
use ast::{
    BinderId, DeclareValue, DependentArgId, InRangeBounds, Literal, NormalExpr, ParamKind,
    ParameterKind, Parameters, TyArg,
};
use ast_format::type_expr::ExprFormatExt;
use diagnostic::Diagnostic;
use diagnostic::code::DomainCode;
use internment::Intern;
use span::SpanTable;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

/// Bundles the context needed to resolve [`TypeExpr`] to [`Ty`].
///
/// Pass this through the resolution tree instead of threading individual parameters.
/// The `subst`, `tag_params`, and `tag_decls` fields are optional — most callers
/// only need `tag_types`.
pub struct TypeEnv<'a> {
    pub tag_types: &'a HashMap<Intern<String>, Ty>,
    pub registry: Option<&'a crate::TypeRegistry>,
    pub subst: Option<&'a HashMap<Intern<String>, Ty>>,
    pub tag_params: Option<&'a HashMap<Intern<String>, Parameters>>,
    pub tag_decls: Option<&'a ast::TagMap>,
    fallback_tag_decls: Option<&'a ast::TagMap>,
    pub dependent_binder: Option<BinderId>,
    type_reads: Option<&'a RefCell<HashSet<Intern<String>>>>,
}

impl<'a> TypeEnv<'a> {
    pub fn new(tag_types: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types,
            registry: None,
            subst: None,
            tag_params: None,
            tag_decls: None,
            fallback_tag_decls: None,
            dependent_binder: None,
            type_reads: None,
        }
    }

    pub(crate) fn with_type_reads(mut self, reads: &'a RefCell<HashSet<Intern<String>>>) -> Self {
        self.type_reads = Some(reads);
        self
    }

    fn tag_type(&self, name: &Intern<String>) -> Option<&Ty> {
        if let Some(reads) = self.type_reads {
            reads.borrow_mut().insert(*name);
        }
        self.tag_types.get(name)
    }

    pub fn with_registry(mut self, registry: &'a crate::TypeRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_subst(&self, subst: &'a HashMap<Intern<String>, Ty>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            registry: self.registry,
            subst: Some(subst),
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            fallback_tag_decls: self.fallback_tag_decls,
            dependent_binder: self.dependent_binder,
            type_reads: self.type_reads,
        }
    }

    pub fn with_tag_params(&self, tag_params: &'a HashMap<Intern<String>, Parameters>) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            registry: self.registry,
            subst: self.subst,
            tag_params: Some(tag_params),
            tag_decls: self.tag_decls,
            fallback_tag_decls: self.fallback_tag_decls,
            dependent_binder: self.dependent_binder,
            type_reads: self.type_reads,
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
                registry: self.registry,
                subst: self.subst,
                tag_params: None,
                tag_decls: self.tag_decls,
                fallback_tag_decls: self.fallback_tag_decls,
                dependent_binder: self.dependent_binder,
                type_reads: self.type_reads,
            },
        }
    }

    pub fn with_tag_decls(&self, tag_decls: &'a ast::TagMap) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            registry: self.registry,
            subst: self.subst,
            tag_params: self.tag_params,
            tag_decls: Some(tag_decls),
            fallback_tag_decls: None,
            dependent_binder: self.dependent_binder,
            type_reads: self.type_reads,
        }
    }

    pub(crate) fn with_tag_declaration_lookup(&self, declarations: TagDeclarations<'a>) -> Self {
        let mut env = self.with_tag_decls(declarations.local);
        env.fallback_tag_decls = declarations.package;
        env
    }

    fn tag_declarations(&self) -> Option<TagDeclarations<'a>> {
        self.tag_decls.map(|local| TagDeclarations {
            local,
            package: self.fallback_tag_decls,
        })
    }

    pub fn with_dependent_binder(&self, dependent_binder: BinderId) -> Self {
        TypeEnv {
            tag_types: self.tag_types,
            registry: self.registry,
            subst: self.subst,
            tag_params: self.tag_params,
            tag_decls: self.tag_decls,
            fallback_tag_decls: self.fallback_tag_decls,
            dependent_binder: Some(dependent_binder),
            type_reads: self.type_reads,
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
        let fixed_array_length_param = self
            .tag_declarations()
            .and_then(|tag_decls| tag_decls.get(tag_name))
            .and_then(|declare| declare.attributes.fixed_array.as_ref())
            .map(|former| former.length_parameter);
        let is_fixed_array_length =
            |name: &Intern<String>, fixed_array_length_param: &Option<Intern<String>>| {
                fixed_array_length_param.is_some_and(|length| length == *name)
            };

        if let Some(tag_params) = self.tag_params
            && let Some(decl_params) = tag_params.get(tag_name)
        {
            let declared: Vec<(Intern<String>, ParamKind)> = decl_params
                .iter()
                .map(|(n, k)| (*n, param_kind_from_decl_parameter_kind(&k.kind)))
                .collect();
            let decl_entries: Vec<_> = decl_params.iter().collect();
            let mut supplied_names = HashSet::new();
            for (i, (name, kind)) in use_site_params.iter().enumerate() {
                let decl_name = decl_entries
                    .iter()
                    .find(|(decl_name, _)| **decl_name == *name)
                    .map(|(decl_name, _)| **decl_name)
                    .or_else(|| decl_entries.get(i).map(|(decl_name, _)| **decl_name));
                if let Some(decl_name) = decl_name {
                    supplied_names.insert(decl_name);
                    match kind {
                        ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => {
                            let is_length_parameter =
                                is_fixed_array_length(&decl_name, &fixed_array_length_param);
                            if is_length_parameter {
                                if let Some(cv) =
                                    size_expr_to_normal(&sp.value, SizeCallName::Mangled, true)
                                {
                                    consts.insert(decl_name, cv);
                                }
                            } else if is_literal_expr(&sp.value) {
                                if let Some(cv) = literal_expr_as_const(&sp.value) {
                                    consts.insert(decl_name, cv);
                                }
                            } else if expr_is_type_surface(&sp.value) {
                                let ty = self.resolve_expr(&sp.value);
                                types.insert(decl_name, ty);
                            }
                        }
                        ParameterKind::Proposition(predicate) => {
                            types.insert(decl_name, Ty::Proposition(predicate.clone()));
                        }
                        ParameterKind::Generic => {
                            types.insert(decl_name, Ty::Opaque(*name));
                        }
                        ParameterKind::Default(expr) => {
                            let value_parameter =
                                is_fixed_array_length(&decl_name, &fixed_array_length_param)
                                    || declared.iter().any(|(name, kind)| {
                                        *name == decl_name && matches!(kind, ParamKind::Value(_))
                                    });
                            if value_parameter {
                                if let Some(value) =
                                    size_expr_to_normal(&expr.value, SizeCallName::Mangled, true)
                                {
                                    consts.insert(decl_name, value);
                                }
                            } else if expr_is_type_surface(&expr.value) {
                                types.insert(decl_name, self.resolve_expr(&expr.value));
                            } else if let Some(value) =
                                size_expr_to_normal(&expr.value, SizeCallName::Mangled, true)
                            {
                                consts.insert(decl_name, value);
                            }
                        }
                        ParameterKind::Inferred { .. } => continue,
                    }
                }
            }
            for (ordinal, (decl_name, decl_kind)) in decl_entries.iter().enumerate() {
                if supplied_names.contains(*decl_name) {
                    continue;
                }
                match &decl_kind.kind {
                    ParameterKind::Default(expr) => {
                        if expr_is_type_surface(&expr.value) {
                            types.insert(**decl_name, self.resolve_expr(&expr.value));
                        } else if let Some(value) =
                            size_expr_to_normal(&expr.value, SizeCallName::Mangled, true)
                        {
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
            for (index, (name, kind)) in use_site_params.iter().enumerate() {
                match kind {
                    ParameterKind::Tagged(sp) | ParameterKind::ValueParam { ty: sp } => {
                        let is_length_parameter = fixed_array_length_param.is_some() && index == 1;
                        if is_length_parameter {
                            if let Some(cv) =
                                size_expr_to_normal(&sp.value, SizeCallName::Mangled, true)
                                && let Some(length_name) = fixed_array_length_param
                            {
                                consts.insert(length_name, cv);
                            }
                        } else if is_literal_expr(&sp.value) {
                            if let Some(cv) = literal_expr_as_const(&sp.value) {
                                consts.insert(*name, cv);
                            }
                        } else if expr_is_type_surface(&sp.value) {
                            let ty = self.resolve_expr(&sp.value);
                            types.insert(*name, ty);
                        }
                    }
                    ParameterKind::Proposition(predicate) => {
                        types.insert(*name, Ty::Proposition(predicate.clone()));
                    }
                    ParameterKind::Generic => {
                        types.insert(*name, Ty::Opaque(*name));
                    }
                    ParameterKind::Default(expr) => {
                        if expr_is_type_surface(&expr.value) {
                            types.insert(*name, self.resolve_expr(&expr.value));
                        } else if let Some(value) =
                            size_expr_to_normal(&expr.value, SizeCallName::Mangled, true)
                        {
                            consts.insert(*name, value);
                        }
                    }
                    ParameterKind::Inferred { .. } => continue,
                }
            }
        }
        DepSubst::from_maps(types, consts)
    }

    pub fn resolve(&self, expr: &ast::Expr) -> Ty {
        match expr {
            ast::Expr::Binary(binary) if binary.op == ast::BinOp::LogicalOr => {
                self.resolve_sum(expr)
            }
            ast::Expr::AnonymousTag(name) => {
                if let Some(subst) = self.subst
                    && let Some(t) = subst.get(name)
                {
                    return t.clone();
                }
                if name.as_str().starts_with_lowercase() {
                    return Ty::Opaque(*name);
                }
                if self
                    .tag_params
                    .and_then(|params| params.get(name))
                    .is_some_and(|params| {
                        !params.is_empty()
                            && params
                                .values()
                                .all(|param| matches!(param.kind, ParameterKind::Default(_)))
                    })
                {
                    return self.resolve_generic(name, &[]);
                }
                self.tag_type(name).cloned().unwrap_or(Ty::Opaque(*name))
            }
            ast::Expr::TagCall(call) => {
                let params = self.generic_params_from_call(&call.name, &call.args);
                self.resolve_generic(&call.name, &params)
            }
            ast::Expr::Ref { inner, mutable, .. } => Ty::Ref {
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
                    self.tag_type(&call.path.value.root)
                        .cloned()
                        .unwrap_or(Ty::Opaque(call.path.value.root))
                }
            }
            ast::Expr::ResultValues(values) if values.is_empty() => Ty::Unit,
            ast::Expr::Lit(_) => Ty::Opaque(Intern::new(String::new())),
            ast::Expr::FixedArrayAlloc { init, size } => Ty::Array {
                elem: Box::new(self.resolve_expr(&init.value)),
                size: size_expr_to_normal(&size.value, SizeCallName::Mangled, true)
                    .unwrap_or_else(|| NormalExpr::Var(Intern::from_ref("<array-size>"))),
            },
            ast::Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => Ty::sequence(
                elems
                    .iter()
                    .map(|elem| ast::ty::SequenceTypeElement {
                        name: elem.name,
                        ty: Box::new(self.resolve_expr(&elem.value)),
                    })
                    .collect(),
            ),
            ast::Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => Ty::Array {
                elem: Box::new(self.resolve_expr(&seed.value)),
                size: size_expr_to_normal(&count.value, SizeCallName::Mangled, true)
                    .unwrap_or_else(|| NormalExpr::Var(Intern::from_ref("<array-size>"))),
            },
            ast::Expr::TypeValue(ty) => (**ty).clone(),
            ast::Expr::DomainValue(domain) => Ty::Literal(ast::ConstValue::Domain(domain.clone())),
            ast::Expr::IntegerDomain(inner) => self.resolve_integer_domain(&inner.value),
            ast::Expr::Range(range) => match (&range.start.value, &range.end.value) {
                (ast::Expr::AnonymousTag(start), ast::Expr::AnonymousTag(end)) if start == end => {
                    self.resolve_integer_domain(&range.start.value)
                }
                (
                    ast::Expr::Lit(ast::Literal::Int(start)),
                    ast::Expr::Lit(ast::Literal::Int(end)),
                ) => resolve_in_range_bounds(&InRangeBounds::Literal(*start, *end), self.tag_types),
                _ => Ty::Opaque(Intern::from_ref("<expr>")),
            },
            _ => Ty::Opaque(Intern::from_ref("<expr>")),
        }
    }

    fn resolve_integer_domain(&self, expr: &ast::Expr) -> Ty {
        let ty = self.resolve_expr(expr);
        if let Ty::Literal(ast::ConstValue::Domain(domain)) = &ty {
            return Ty::AnonymousInteger {
                validity: ast::integer::IntegerValidity::new((**domain).clone()),
            };
        }
        self.registry
            .and_then(|registry| registry.integer_validity_for_type(&ty))
            .or_else(|| ty.anonymous_integer_validity())
            .map(|validity| Ty::AnonymousInteger {
                validity: validity.clone(),
            })
            .unwrap_or_else(|| Ty::Opaque(Intern::from_ref("<non-integer-domain>")))
    }

    pub(crate) fn integer_domain_diagnostic(
        &self,
        surface: &ast::Spanned<ast::Expr>,
        spans: &SpanTable,
    ) -> Option<Diagnostic> {
        let ast::Expr::IntegerDomain(inner) = &surface.value else {
            return None;
        };
        if !matches!(self.resolve_integer_domain(&inner.value), Ty::Opaque(name) if name.as_str() == "<non-integer-domain>")
        {
            return None;
        }
        let ty = self.resolve_expr(&inner.value);
        let name = match &ty {
            Ty::Record { name, .. } => name.to_string(),
            _ => ty.format_for_hover(),
        };
        let is_record = self.registry.map_or_else(
            || ty.is_record(),
            |registry| registry.resolved_definition_for_type(&ty).is_record(),
        );
        let operand = inner.value.format_surface();
        let actual = if is_record {
            "record type"
        } else {
            "non-integer type"
        };
        let reason = if operand == name {
            format!("`{name}` is a {actual}")
        } else {
            format!("`{operand}` resolves to {actual} `{name}`")
        };
        Some(
            // type-non-integer-domain
            Diagnostic::new(
                DomainCode::NonInteger,
                format!("`is in` requires an integer domain, but {reason}"),
            )
            .with_arg("type", name)
            .with_help(if is_record {
                "records have no integer domain; use the record type directly and implement operations using its fields"
            } else {
                "use an integer type or an integer domain after `is in`"
            })
            .at_span_id(inner.span_id, spans),
        )
    }

    pub(crate) fn resolve_result_alternatives(
        &self,
        alternatives: &[ast::Spanned<ast::ResultAlternative>],
    ) -> Option<Ty> {
        if alternatives.len() < 2
            || alternatives
                .iter()
                .any(|alternative| alternative.value.proposition.is_some())
            || !alternatives.iter().any(|alternative| {
                self.tag_types.contains_key(&alternative.value.label)
                    || self
                        .subst
                        .is_some_and(|subst| subst.contains_key(&alternative.value.label))
            })
        {
            return None;
        }
        let surface = alternatives
            .iter()
            .map(|alternative| {
                ast::Typed::infer(
                    ast::Expr::AnonymousTag(alternative.value.label),
                    alternative.span_id,
                )
            })
            .reduce(|lhs, rhs| {
                let span_id = lhs.span_id;
                ast::Typed::infer(
                    ast::Expr::Binary(ast::Binary::new(lhs, ast::BinOp::LogicalOr, rhs)),
                    span_id,
                )
            })?;
        Some(self.resolve(&surface.value))
    }

    fn resolve_sum(&self, expr: &ast::Expr) -> Ty {
        fn alternatives<'a>(expr: &'a ast::Expr, out: &mut Vec<&'a ast::Expr>) {
            if let ast::Expr::Binary(binary) = expr
                && binary.op == ast::BinOp::LogicalOr
            {
                alternatives(&binary.lhs.value, out);
                alternatives(&binary.rhs.value, out);
            } else {
                out.push(expr);
            }
        }
        let mut members = Vec::new();
        alternatives(expr, &mut members);
        let variants: Vec<ast::ty::UnionVariant> = members
            .into_iter()
            .map(|member| match member {
                ast::Expr::AnonymousTag(name) if !self.tag_types.contains_key(name) => {
                    ast::ty::UnionVariant::new(*name, Vec::new())
                }
                ast::Expr::TagCall(call) if !self.tag_types.contains_key(&call.name) => {
                    ast::ty::UnionVariant::new(
                        call.name,
                        call.args
                            .iter()
                            .enumerate()
                            .map(|(index, arg)| {
                                (
                                    Intern::new(format!("field_{index}")),
                                    Box::new(self.resolve_expr(&arg.value)),
                                )
                            })
                            .collect(),
                    )
                }
                _ => {
                    let ty = self.resolve_expr(member);
                    let mut variant = ast::ty::UnionVariant::new(
                        ty.type_name()
                            .copied()
                            .unwrap_or_else(|| Intern::new(member.format_surface())),
                        vec![(Intern::from_ref("value"), Box::new(ty))],
                    );
                    variant.kind = ast::ty::UnionVariantKind::Type;
                    variant
                }
            })
            .collect();
        union_type(variants)
    }

    pub fn resolve_expr(&self, expr: &ast::Expr) -> Ty {
        if let Some(declarations) = self.tag_declarations() {
            let domains = crate::domain::Domains::new(
                self.with_tag_declaration_lookup(declarations),
                declarations,
            );
            if matches!(expr, ast::Expr::TagCall(call) if declarations.get(&call.name).and_then(|d| d.params.as_ref()).is_some_and(|p| p.values().any(|p| p.range_pattern.is_some())))
                && let Ok(Some(value)) = domains.value(expr)
            {
                return Ty::Literal(value);
            }
        }
        self.resolve(expr)
    }

    fn generic_params_from_call(
        &self,
        name: &Intern<String>,
        args: &[ast::Typed<ast::Expr>],
    ) -> Vec<(Intern<String>, ParameterKind)> {
        let raw: Vec<(Option<Intern<String>>, ast::Typed<ast::Expr>)> = args
            .iter()
            .map(|arg| {
                named_generic_argument(arg)
                    .map_or_else(|| (None, arg.clone()), |(name, value)| (Some(name), value))
            })
            .collect();

        let Some(declaration_params) = self.tag_params.and_then(|tag_params| tag_params.get(name))
        else {
            return raw
                .into_iter()
                .enumerate()
                .map(|(index, (name, value))| {
                    (
                        name.unwrap_or_else(|| Intern::new(format!("__expr_{index}"))),
                        ParameterKind::Default(Box::new(value)),
                    )
                })
                .collect();
        };

        let declaration_names: Vec<Intern<String>> = declaration_params.keys().copied().collect();
        if raw.iter().all(|(name, _)| name.is_none()) {
            return raw
                .into_iter()
                .enumerate()
                .map(|(index, (_, value))| {
                    (
                        declaration_names
                            .get(index)
                            .copied()
                            .unwrap_or_else(|| Intern::new(format!("__expr_{index}"))),
                        ParameterKind::Default(Box::new(value)),
                    )
                })
                .collect();
        }

        let mut slots: Vec<Option<(Intern<String>, ast::Typed<ast::Expr>)>> =
            vec![None; declaration_names.len()];
        let mut next_positional = 0;
        for (arg_name, value) in raw {
            let index = arg_name
                .and_then(|arg_name| {
                    declaration_names
                        .iter()
                        .position(|decl_name| *decl_name == arg_name)
                })
                .unwrap_or_else(|| {
                    let index = next_positional;
                    next_positional += 1;
                    index
                });
            if let Some(slot) = slots.get_mut(index)
                && slot.is_none()
            {
                let declaration_name = declaration_names[index];
                *slot = Some((declaration_name, value));
            }
        }

        slots
            .into_iter()
            .flatten()
            .map(|(name, value)| (name, ParameterKind::Default(Box::new(value))))
            .collect()
    }

    pub(crate) fn resolve_generic(
        &self,
        name: &Intern<String>,
        params: &[(Intern<String>, ParameterKind)],
    ) -> Ty {
        if let Some(subst) = self.subst
            && params
                .iter()
                .all(|(_, kind)| matches!(kind, ParameterKind::Generic))
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
            && let Some(tag_decls) = self.tag_declarations()
            && let Some(ast::DeclareValue::Has(members)) = tag_decls.get(name).map(|d| &d.value)
        {
            if let Some(Ty::Named {
                instance,
                name: display_name,
                ..
            }) = self.tag_type(name)
            {
                return Ty::Named {
                    instance: ast::ty::NamedTypeInstance {
                        declaration: instance.declaration,
                        arguments: self
                            .resolved_params(name, params, &local_dep)
                            .unwrap_or_default(),
                    },
                    name: *display_name,
                };
            }
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
            && let Some(tag_decls) = self.tag_declarations()
            && let Some(ast::DeclareValue::Alias(sp)) = tag_decls.get(name).map(|d| &d.value)
        {
            return with_subst.resolve_expr(&sp.value);
        }
        let base = self.tag_type(name).cloned().unwrap_or(Ty::Opaque(*name));
        if merged_types.is_empty() && local_dep.consts.is_empty() {
            base
        } else {
            let subst = DepSubst::from_maps(merged_types, local_dep.consts);
            let mut resolved = subst.apply_to_ty(&base);
            let arguments = self.resolved_params(name, params, &subst);
            if let Ty::Named { instance, .. } = &mut resolved {
                instance.arguments = arguments.clone().unwrap_or_default();
            }
            if let Ty::Union {
                ref mut resolved_params,
                ..
            } = resolved
            {
                *resolved_params = arguments.clone();
            }
            if let Ty::Record {
                ref mut resolved_params,
                ..
            } = resolved
            {
                *resolved_params = arguments;
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
        InRangeBounds::LiteralToTag(_, name) => Ty::Opaque(*name),
        InRangeBounds::Tag(name) => {
            if let Some(ty) = tag_types.get(name) {
                if ty.is_bounded_int() {
                    return ty.clone();
                }
                if let Some(b) = ty.scalar_bounds_from_range_value() {
                    return Ty::bounded_int(i256::I256::from(b.min), i256::I256::from(b.max));
                }
            }
            Ty::Opaque(*name)
        }
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
    check_type_application_with_defaults(declared_params, provided_params, 0)
}

fn check_type_application_with_defaults(
    declared_params: &[(Intern<String>, ParamKind)],
    provided_params: &[(Intern<String>, ParameterKind)],
    defaulted_params: usize,
) -> Result<(), String> {
    let required_params = declared_params.len().saturating_sub(defaulted_params);
    if provided_params.len() < required_params || provided_params.len() > declared_params.len() {
        return Err(if defaulted_params == 0 {
            format!(
                "expected {} type arguments, found {}",
                declared_params.len(),
                provided_params.len()
            )
        } else {
            format!(
                "expected between {} and {} type arguments, found {}",
                required_params,
                declared_params.len(),
                provided_params.len()
            )
        });
    }

    for ((decl_name, decl_kind), (_, prov_kind)) in
        declared_params.iter().zip(provided_params.iter())
    {
        match (decl_kind, prov_kind) {
            (ParamKind::Type, ParameterKind::Tagged(_))
            | (ParamKind::Type, ParameterKind::Generic)
            | (ParamKind::Type, ParameterKind::Default(_)) => {}
            (ParamKind::Type, ParameterKind::Proposition(_))
            | (ParamKind::Type, ParameterKind::ValueParam { .. })
            | (ParamKind::Type, ParameterKind::Inferred { .. }) => {
                return Err(format!(
                    "expected type argument for `{}`, found const value",
                    decl_name.as_str()
                ));
            }
            (ParamKind::Value(_), ParameterKind::Proposition(_))
            | (ParamKind::Value(_), ParameterKind::Tagged(_))
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
        ParameterKind::Proposition(predicate) => {
            ParamKind::Value(Box::new(Ty::Proposition(predicate.clone())))
        }
        ParameterKind::Generic => ParamKind::Type,
        ParameterKind::Default(expr) if expr_is_type_surface(&expr.value) => ParamKind::Type,
        ParameterKind::Default(_) => ParamKind::Value(Box::new(crate::ty::Ty::i64())),
        ParameterKind::Tagged(_)
        | ParameterKind::ValueParam { .. }
        | ParameterKind::Inferred { .. } => ParamKind::Value(Box::new(crate::ty::Ty::i64())),
    }
}

fn is_literal_expr(expr: &ast::Expr) -> bool {
    matches!(expr, ast::Expr::Lit(Literal::Int(_)))
}

fn named_generic_argument(
    arg: &ast::Typed<ast::Expr>,
) -> Option<(Intern<String>, ast::Typed<ast::Expr>)> {
    let ast::Expr::Bind(bind) = &arg.value else {
        return None;
    };
    if bind.params.is_some() {
        return None;
    }
    let ast::BindValue::Expr(value) = &bind.value else {
        return None;
    };
    Some((bind.name, (**value).clone()))
}

fn literal_expr_as_const(expr: &ast::Expr) -> Option<NormalExpr> {
    match expr {
        ast::Expr::Lit(Literal::Int(n)) => Some(NormalExpr::from(*n)),
        _ => None,
    }
}

pub(crate) fn expr_is_type_surface(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::ResultValues(values) if values.is_empty() => true,
        ast::Expr::Binary(binary) if binary.op == ast::BinOp::LogicalOr => {
            expr_is_type_surface(&binary.lhs.value) && expr_is_type_surface(&binary.rhs.value)
        }
        ast::Expr::TypeValue(_)
        | ast::Expr::IntegerDomain(_)
        | ast::Expr::AnonymousTag(_)
        | ast::Expr::TagCall(_)
        | ast::Expr::FnCall(ast::FnCall { args: None, .. }) => true,
        ast::Expr::Ref { inner, .. } | ast::Expr::TakePtr(inner) => {
            expr_is_type_surface(&inner.value)
        }
        ast::Expr::Range(range) => match (&range.start.value, &range.end.value) {
            (ast::Expr::AnonymousTag(start), ast::Expr::AnonymousTag(end)) if start == end => true,
            (ast::Expr::Lit(ast::Literal::Int(_)), ast::Expr::Lit(ast::Literal::Int(_))) => true,
            _ => false,
        },
        ast::Expr::FixedArrayAlloc { init, size } => {
            expr_is_type_surface(&init.value)
                && size_expr_to_normal(&size.value, SizeCallName::Mangled, true).is_some()
        }
        ast::Expr::SequenceLiteral(ast::SequenceLiteral::Elements(elems)) => {
            elems.iter().all(|elem| expr_is_type_surface(&elem.value))
        }
        ast::Expr::SequenceLiteral(ast::SequenceLiteral::Repeat { seed, count }) => {
            expr_is_type_surface(&seed.value)
                && size_expr_to_normal(&count.value, SizeCallName::Mangled, true).is_some()
        }
        _ => false,
    }
}

pub(crate) fn typevars_from_receiver_expr(expr: &ast::Expr) -> HashMap<Intern<String>, Ty> {
    let mut out = HashMap::new();
    let ast::Expr::TagCall(call) = expr else {
        return out;
    };
    for arg in &call.args {
        if let ast::Expr::AnonymousTag(name) = arg.value
            && name.as_str().starts_with_lowercase()
        {
            out.insert(name, Ty::Opaque(name));
        }
    }
    out
}

#[cfg(test)]
#[path = "type_surface_tests.rs"]
mod nominal_tests;
