use super::*;

pub(crate) fn param_kind_surface_for_hover(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Tagged(sp) => sp.value.format_surface(),
        ParameterKind::ValueParam { ty } => ty.value.format_surface(),
        ParameterKind::Inferred { ty } => {
            format!("{}: ?", ty.value.format_surface())
        }
        ParameterKind::Proposition(predicate) => format!("is {predicate}"),
        ParameterKind::Generic => String::new(),
        ParameterKind::Default(expr) => format!(": {:?}", expr.value),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub markdown: String,
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub module_prefix: Option<String>,
}

impl HoverResult {
    pub fn single(markdown: String) -> Self {
        Self {
            markdown,
            module_prefix: None,
        }
    }

    /// Prefix hover with `{module}` when the tag's declaring module is known.
    pub fn with_tag_module(
        self,
        package: Option<&PackageSemanticIndex>,
        tag_name: &Intern<String>,
    ) -> Self {
        let prefix = package.and_then(|pkg| pkg.tag_module.get(tag_name).cloned());
        self.with_prefix(prefix)
    }

    /// Prefix hover with `{module}.{def}` when the definition's declaring module is known.
    pub fn with_def_module(
        self,
        package: Option<&PackageSemanticIndex>,
        def_name: &Intern<String>,
    ) -> Self {
        let prefix = package.and_then(|pkg| pkg.qualified_def_path(def_name));
        self.with_prefix(prefix)
    }

    /// Prefix hover with `{module}.{union}` when the union's declaring module is known.
    pub fn with_qualified_union(
        self,
        package: Option<&PackageSemanticIndex>,
        union_name: &Intern<String>,
    ) -> Self {
        let prefix = package.and_then(|pkg| pkg.qualified_union_path(union_name));
        self.with_prefix(prefix)
    }

    fn with_prefix(mut self, prefix: Option<String>) -> Self {
        if self.module_prefix.is_none()
            && let Some(prefix) = prefix
        {
            self.module_prefix = Some(prefix);
        }
        self
    }
}

/// Cross-file tag and variant index for package-scoped IDE hover.
#[derive(Clone, PartialEq)]
pub struct PackageSemanticIndex {
    pub type_registry: crate::TypeRegistry,
    pub tag_types: HashMap<Intern<String>, Ty>,
    pub tag_params: HashMap<Intern<String>, Parameters>,
    pub variant_map: VariantMap,
    /// Declaring module per tag name (`Bool` → `core.primitive`).
    /// Qualified path to the base type (e.g. `core.primitive.Bool` for variant `False`).
    pub tag_module: HashMap<Intern<String>, String>,
    /// Tag declarations from across the package for cross-file name hovers.
    pub tag_decls: HashMap<Intern<String>, TypedTag>,
    /// Source signatures for declared member functions, keyed by method name.
    pub member_functions: HashMap<Intern<String>, Vec<(Intern<String>, ast::HasFunction)>>,
    member_definitions: HashMap<DefId, (Intern<String>, Intern<String>, SpanId)>,
    /// Declaring module per definition name (`Default` → `core.default`).
    pub def_module: HashMap<Intern<String>, String>,
    /// Merged module-level doc comments (`--|`), keyed by qualified module path.
    /// Populated by [`stage_build_index`] — concatenates docs from all files in
    /// each module directory in alphabetical filename order.
    pub module_docs: HashMap<String, String>,
}

impl PackageSemanticIndex {
    pub(super) fn member_function_for_definition(
        &self,
        id: &DefId,
    ) -> Option<(Intern<String>, &ast::HasFunction)> {
        let (owner, name, span) = self.member_definitions.get(id)?;
        self.member_functions
            .get(name)?
            .iter()
            .find_map(|(candidate, function)| {
                (*candidate == *owner && function.name_span == *span).then_some((*owner, function))
            })
    }

    pub fn tag_decl_for_word(&self, name: &Intern<String>) -> Option<(&String, &TypedTag)> {
        self.tag_module.get(name).zip(self.tag_decls.get(name))
    }
}

impl PackageSemanticIndex {
    pub fn from_typed_asts(asts: &[&TypedFileAst]) -> Self {
        let mut tag_types: HashMap<Intern<String>, Ty> = HashMap::new();
        let mut tag_params: HashMap<Intern<String>, Parameters> = HashMap::new();
        let mut type_registry = crate::TypeRegistry::default();
        let mut tag_module = HashMap::new();
        let mut tag_decls = HashMap::new();
        let mut member_functions =
            HashMap::<Intern<String>, Vec<(Intern<String>, ast::HasFunction)>>::new();
        let mut member_definitions = HashMap::new();
        let mut def_module = HashMap::new();
        let mut module_docs: HashMap<String, Vec<String>> = HashMap::new();

        for ast in asts {
            type_registry.merge_from(&ast.type_registry);
            for id in ast.defs.keys() {
                if let Some((owner, function)) = ast.member_function_for_definition(id) {
                    member_definitions.insert(*id, (owner, function.name, function.name_span));
                }
            }
            for (owner, declaration) in &ast.eval_ast.tags {
                if let ast::DeclareValue::Has(members) = &declaration.value {
                    for member in members {
                        if let ast::HasMember::Function(function) = member {
                            member_functions
                                .entry(function.name)
                                .or_default()
                                .push((*owner, function.as_ref().clone()));
                        }
                    }
                }
            }
            for (tag_id, ty) in ast.tag_types.iter() {
                tag_types.insert(tag_id.0, ty.clone());
            }
            for (tag_id, tag) in &*ast.tags {
                if let Some(params) = &tag.params {
                    tag_params.insert(tag_id.0, params.clone());
                }
            }
            if let Some(origin) = ast.semantic_origin.as_ref() {
                let simple_module = origin
                    .module
                    .iter()
                    .map(|part| part.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                let module = if simple_module.is_empty() {
                    origin.package.name.as_str().to_string()
                } else {
                    format!("{}.{}", origin.package.name.as_str(), simple_module)
                };
                for (tag_id, tag) in &*ast.tags {
                    tag_module.entry(tag_id.0).or_insert_with(|| module.clone());
                    tag_decls.insert(tag_id.0, tag.clone());
                }
                for def_id in ast.defs.keys() {
                    def_module.entry(def_id.0).or_insert_with(|| module.clone());
                }
                if let Some(doc) = ast.module_doc.as_ref() {
                    module_docs
                        .entry(module)
                        .or_default()
                        .push(doc.value.clone());
                    if !simple_module.is_empty() {
                        module_docs
                            .entry(simple_module)
                            .or_default()
                            .push(doc.value.clone());
                    }
                }
            }
        }

        let variant_map = collect_package_variant_map(asts);

        Self {
            type_registry,
            tag_types,
            tag_params,
            variant_map,
            tag_module,
            tag_decls,
            member_functions,
            member_definitions,
            def_module,
            module_docs: module_docs
                .into_iter()
                .map(|(module, docs)| (module, docs.join("\n\n")))
                .collect(),
        }
    }

    /// `core.primitive.Bool` for union `Bool` declared in module `core.primitive`.
    pub fn qualified_union_path(&self, union: &Intern<String>) -> Option<String> {
        let module = self.tag_module.get(union)?;
        Some(format!("{module}.{}", union.as_str()))
    }

    /// `core.default` for definition `Default` declared in module `core.default`.
    pub fn qualified_def_path(&self, def: &Intern<String>) -> Option<String> {
        self.def_module.get(def).cloned()
    }
}

pub(crate) fn const_union_variant_label(ty: &Ty, discriminant: usize) -> Option<String> {
    let values = ty.union_literal_values()?;
    let cv = values.get(discriminant)?;
    match cv {
        ast::ConstValue::Text(s) => Some(format!("'{s}'")),
        _ => Some(cv.to_hover_string()),
    }
}

/// Tag metadata lookup shared by package-level and per-file hover formatting.
pub(crate) trait TagLookup {
    fn ty_of(&self, name: &Intern<String>) -> Option<&Ty>;
    fn params_of(&self, name: &Intern<String>) -> Option<&Parameters>;
    fn tag_name_for_ty(&self, ty: &Ty) -> Option<&str>;
}

impl TagLookup for TypedFileAst {
    fn ty_of(&self, name: &Intern<String>) -> Option<&Ty> {
        self.tag_types.get(&TagId(*name))
    }

    fn params_of(&self, name: &Intern<String>) -> Option<&Parameters> {
        self.tags
            .get(&TagId(*name))
            .and_then(|tag| tag.params.as_ref())
    }

    fn tag_name_for_ty(&self, ty: &Ty) -> Option<&str> {
        self.tag_types
            .iter()
            .find(|(_, tag_ty)| *tag_ty == ty)
            .map(|(id, _)| id.0.as_str())
    }
}

impl<'a> TagLookup
    for (
        &'a HashMap<Intern<String>, Ty>,
        Option<&'a HashMap<Intern<String>, Parameters>>,
    )
{
    fn ty_of(&self, name: &Intern<String>) -> Option<&Ty> {
        self.0.get(name)
    }

    fn params_of(&self, name: &Intern<String>) -> Option<&Parameters> {
        self.1.and_then(|params| params.get(name))
    }

    fn tag_name_for_ty(&self, ty: &Ty) -> Option<&str> {
        self.0
            .iter()
            .find(|(_, tag_ty)| *tag_ty == ty)
            .map(|(name, _)| name.as_str())
    }
}

pub(crate) fn format_variant_pattern_label(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tags: &impl TagLookup,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = type_annotation_surface_for_hover(t, tags);
            if type_str == n.as_str() {
                // Opaque generic matching the field name — show just the name.
                n.as_str().to_string()
            } else {
                format!("{} {}", n.as_str(), type_str)
            }
        })
        .collect();
    format!("{}({})", name, parts.join(", "))
}

/// Type name as written in a `name Type` declare (e.g. `List(NamedTy)` not `pointer: …`).
pub(crate) fn type_annotation_surface_for_hover(ty: &Ty, tags: &impl TagLookup) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some(tag_name) = tags.tag_name_for_ty(ty)
            {
                return tag_name.to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record {
            name,
            resolved_params,
            ..
        } => {
            if let Some(surface) = generic_record_surface(name, resolved_params.as_deref(), tags) {
                return surface;
            }
            if tags.ty_of(name).is_some() && !tag_has_generic_params(name, tags) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => format_ty_for_hover(other),
    }
}

fn tag_has_generic_params(name: &Intern<String>, tags: &impl TagLookup) -> bool {
    tags.params_of(name).is_some_and(|params| {
        params
            .iter()
            .any(|(_, kind)| matches!(kind.kind, ParameterKind::Generic))
    })
}

fn generic_record_surface(
    name: &Intern<String>,
    resolved_params: Option<&[(Intern<String>, TyArg)]>,
    tags: &impl TagLookup,
) -> Option<String> {
    if !tag_has_generic_params(name, tags) {
        return None;
    }
    let arguments = resolved_params?
        .iter()
        .map(|(_, argument)| match argument {
            TyArg::Type(ty) => type_annotation_surface_for_hover(ty, tags),
            TyArg::Const(value) => value.to_string(),
        })
        .collect::<Vec<_>>();
    Some(format!("{name}({})", arguments.join(", ")))
}

/// Format a `Ty` for hover display.
pub fn format_ty_for_hover(ty: &Ty) -> String {
    match ty {
        Ty::Named { .. } => ty.format_for_hover(),
        Ty::AnonymousInteger { .. } => ty.format_for_hover(),
        Ty::Proposition(_) | Ty::ResultFamily { .. } => ty.format_for_hover(),
        Ty::Address { .. } => ty.format_for_hover(),
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Integer) => "integer literal".to_string(),
        Ty::UnresolvedLiteral(ast::ty::LiteralKind::Text) => "text literal".to_string(),
        Ty::UnresolvedSequence { .. } => ty.format_for_hover(),
        Ty::Float { value } => {
            if let Some(HashFloat(v)) = value {
                format!("f64 = {}", v)
            } else {
                "f64".to_string()
            }
        }
        Ty::Unit => "()".to_string(),
        Ty::Record { fields, .. } => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(fname, fty)| format!("{}: {}", fname.as_str(), format_ty_for_hover(fty)))
                .collect();
            parts.join(", ")
        }
        Ty::Union { name, .. } => format!("Union({})", name.as_str()),
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Array { elem, size } => format!("[{}; {}]", format_ty_for_hover(elem), size),
        Ty::OwnedStorage { inner } => format!("own {}", format_ty_for_hover(inner)),
        Ty::Ptr { inner } => format!("*{}", format_ty_for_hover(inner)),
        Ty::Ref { inner, mutable } => {
            let prefix = if *mutable { "mut " } else { "ref " };
            format!("{}{}", prefix, format_ty_for_hover(inner))
        }
        Ty::Sequence(elements) => elements
            .iter()
            .map(|element| match element.name {
                Some(name) => format!("{name} {}", format_ty_for_hover(&element.ty)),
                None => format_ty_for_hover(&element.ty),
            })
            .collect::<Vec<_>>()
            .join(", "),
        Ty::Literal(cv) => match cv {
            ast::ConstValue::Text(s) => format!("'{s}'"),
            other => other.to_hover_string(),
        },
    }
}

impl TypedFileAst {
    pub(super) fn result_evidence_hover(&self, expr_id: ExprId, source: &str) -> Option<String> {
        if let Some(EvidenceProposition::IntegerComparison {
            lhs,
            rhs,
            comparison,
        }) = self.exprs.proposition_evidence_of(expr_id)
        {
            let left = self.comparison_operand_surface(*lhs, source)?;
            let right = self.comparison_operand_surface(*rhs, source)?;
            let relation = ast::ProofRelation::from(*comparison);
            return Some(format!("{left} {relation} {right}"));
        }
        let Ty::ResultFamily {
            owner,
            alternatives,
        } = self.exprs.ty_of(expr_id)?
        else {
            return None;
        };
        let evidence = self.exprs.result_evidence_of(expr_id)?;
        if evidence.owner != *owner {
            return None;
        }
        let mut alternatives = alternatives.iter().collect::<Vec<_>>();
        alternatives.sort_by_key(|alternative| alternative.label.as_str() != "True");
        let mut outcomes = Vec::with_capacity(alternatives.len());
        for outcome in alternatives {
            let label = outcome.label.as_str();
            let alternative = evidence
                .alternatives
                .iter()
                .find(|alternative| alternative.label.as_str() == label)?;
            let EvidenceProposition::IntegerComparison {
                lhs,
                rhs,
                comparison,
            } = alternative.proposition;
            let left = self.comparison_operand_surface(lhs, source)?;
            let right = self.comparison_operand_surface(rhs, source)?;
            let relation = ast::ProofRelation::from(comparison);
            outcomes.push(format!("{label} thus {left} {relation} {right}"));
        }
        Some(outcomes.join(" or "))
    }

    fn comparison_operand_surface<'a>(&self, operand: ExprId, source: &'a str) -> Option<&'a str> {
        let span = self.span_table.get(self.exprs.span_of(operand)?);
        let surface = source.get(span.start()..span.end())?.trim();
        (!surface.is_empty()).then_some(surface)
    }
}
