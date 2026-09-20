use super::*;

pub(crate) fn param_kind_surface_for_hover(kind: &ParameterKind) -> String {
    match kind {
        ParameterKind::Tagged(sp) => sp.value.format_surface(),
        ParameterKind::ValueParam { ty } => ty.value.format_surface(),
        ParameterKind::Inferred { ty } => {
            format!("{}: ?", ty.value.format_surface())
        }
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
        mut self,
        package: Option<&PackageSemanticIndex>,
        tag_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(module) = pkg.tag_module.get(tag_name)
        {
            self.module_prefix = Some(module.clone());
        }
        self
    }

    /// Prefix hover with `{module}.{def}` when the definition's declaring module is known.
    pub fn with_def_module(
        mut self,
        package: Option<&PackageSemanticIndex>,
        def_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_def_path(def_name)
        {
            self.module_prefix = Some(path);
        }
        self
    }

    /// Prefix hover with `{module}.{union}` when the union's declaring module is known.
    pub fn with_qualified_union(
        mut self,
        package: Option<&PackageSemanticIndex>,
        union_name: &Intern<String>,
    ) -> Self {
        if self.module_prefix.is_none()
            && let Some(pkg) = package
            && let Some(path) = pkg.qualified_union_path(union_name)
        {
            self.module_prefix = Some(path);
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
    /// Declaring module per definition name (`Default` → `core.default`).
    pub def_module: HashMap<Intern<String>, String>,
    /// Merged module-level doc comments (`--|`), keyed by qualified module path.
    /// Populated by [`stage_build_index`] — concatenates docs from all files in
    /// each module directory in alphabetical filename order.
    pub module_docs: HashMap<String, String>,
}

impl PackageSemanticIndex {
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
        let mut def_module = HashMap::new();
        let mut module_docs: HashMap<String, Vec<String>> = HashMap::new();

        for ast in asts {
            type_registry.merge_from(&ast.type_registry);
            for (tag_id, ty) in &ast.tag_types {
                tag_types.insert(tag_id.0, ty.clone());
            }
            for (tag_id, tag) in &ast.tags {
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
                for (tag_id, tag) in &ast.tags {
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
            def_module,
            module_docs: module_docs
                .into_iter()
                .map(|(module, docs)| (module, docs.join("\n\n")))
                .collect(),
        }
    }

    pub fn module_for_variant_word(&self, variant: &Intern<String>) -> Option<&String> {
        let (union, _, _) = self.variant_map.get(variant)?.first()?;
        self.tag_module.get(union)
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

pub(crate) fn format_variant_pattern_label(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = variant_pattern_field_type_surface(t, tag_types, tag_params);
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

pub(crate) fn format_variant_pattern_label_local(
    name: &str,
    fields: &[(Intern<String>, Ty)],
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    if fields.is_empty() {
        return name.to_string();
    }
    let parts: Vec<String> = fields
        .iter()
        .map(|(n, t)| {
            let type_str = variant_pattern_field_type_surface_local(t, tag_types, tags);
            if type_str == n.as_str() {
                n.as_str().to_string()
            } else {
                format!("{} {}", n.as_str(), type_str)
            }
        })
        .collect();
    format!("{}({})", name, parts.join(", "))
}

pub(crate) fn variant_pattern_field_type_surface(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    if let Ty::Record { name, .. } = ty
        && name.as_str() != "List"
    {
        return format_ty_for_hover(ty).replace("Union(Type)", "Union(union)");
    } else if let Ty::Record { name, .. } = ty
        && name.as_str() == "List"
    {
        return "List".to_string();
    }
    type_annotation_surface_for_hover(ty, tag_types, tag_params)
}

pub(crate) fn variant_pattern_field_type_surface_local(
    ty: &Ty,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    if let Ty::Record { name, .. } = ty
        && name.as_str() != "List"
    {
        return format_ty_for_hover(ty).replace("Union(Type)", "Union(union)");
    } else if let Ty::Record { name, .. } = ty
        && name.as_str() == "List"
    {
        return "List".to_string();
    }
    type_annotation_surface_for_hover_local(ty, tag_types, tags)
}

/// Type name as written in a `name Type` declare (e.g. `List(NamedTy)` not `pointer: …`).
pub(crate) fn type_annotation_surface_for_hover(
    ty: &Ty,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some((tag_name, _)) = tag_types.iter().find(|(_, tag_ty)| *tag_ty == ty)
            {
                return tag_name.as_str().to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => {
            if let Some(surface) = generic_record_surface(
                name,
                fields,
                resolved_params.as_deref(),
                tag_types,
                tag_params,
            ) {
                return surface;
            }
            if tag_types.contains_key(name) && !tag_has_generic_params(name, tag_params) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => format_ty_for_hover(other),
    }
}

pub(crate) fn type_annotation_surface_for_hover_local(
    ty: &Ty,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> String {
    match ty {
        Ty::Union { name, .. } => {
            if name.as_str() == "union"
                && let Some((tag_name, _)) = tag_types.iter().find(|(_, tag_ty)| *tag_ty == ty)
            {
                return tag_name.0.as_str().to_string();
            }
            name.as_str().to_string()
        }
        Ty::Opaque(name) => name.as_str().to_string(),
        Ty::Record {
            name,
            fields,
            resolved_params,
        } => {
            if let Some(surface) = generic_record_surface_local(
                name,
                fields,
                resolved_params.as_deref(),
                tag_types,
                tags,
            ) {
                return surface;
            }
            if tag_types.contains_key(&TagId(*name)) && !tag_has_generic_params_local(name, tags) {
                return name.as_str().to_string();
            }
            format_ty_for_hover(ty)
        }
        other => format_ty_for_hover(other),
    }
}

pub(crate) fn tag_has_generic_params(
    name: &Intern<String>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> bool {
    tag_params
        .and_then(|tp| tp.get(name))
        .is_some_and(|params| {
            params
                .iter()
                .any(|(_, k)| matches!(k.kind, ParameterKind::Generic))
        })
}

pub(crate) fn tag_has_generic_params_local(
    name: &Intern<String>,
    tags: &HashMap<TagId, TypedTag>,
) -> bool {
    tags.get(&TagId(*name))
        .and_then(|tag| tag.params.as_ref())
        .is_some_and(|params| {
            params
                .iter()
                .any(|(_, kind)| matches!(kind.kind, ParameterKind::Generic))
        })
}

pub(crate) fn generic_record_surface(
    name: &Intern<String>,
    fields: &[(Intern<String>, Box<Ty>)],
    resolved_params: Option<&[(Intern<String>, TyArg)]>,
    tag_types: &HashMap<Intern<String>, Ty>,
    tag_params: Option<&HashMap<Intern<String>, Parameters>>,
) -> Option<String> {
    if !tag_has_generic_params(name, tag_params) {
        return None;
    }
    match name.as_str() {
        "List" => {
            let elem = list_element_ty_from_resolved_params(resolved_params)
                .or_else(|| list_element_ty_from_record_fields(fields))?;
            Some(format!(
                "List({})",
                type_annotation_surface_for_hover(&elem, tag_types, tag_params)
            ))
        }
        _ => None,
    }
}

pub(crate) fn generic_record_surface_local(
    name: &Intern<String>,
    fields: &[(Intern<String>, Box<Ty>)],
    resolved_params: Option<&[(Intern<String>, TyArg)]>,
    tag_types: &HashMap<TagId, Ty>,
    tags: &HashMap<TagId, TypedTag>,
) -> Option<String> {
    if !tag_has_generic_params_local(name, tags) {
        return None;
    }
    match name.as_str() {
        "List" => Some(format!(
            "List({})",
            type_annotation_surface_for_hover_local(
                &list_element_ty_from_resolved_params(resolved_params)
                    .or_else(|| list_element_ty_from_record_fields(fields))?,
                tag_types,
                tags,
            )
        )),
        _ => None,
    }
}

pub(crate) fn list_element_ty_from_resolved_params(
    params: Option<&[(Intern<String>, TyArg)]>,
) -> Option<Ty> {
    params?.iter().find_map(|(_, arg)| match arg {
        TyArg::Type(ty) => Some((**ty).clone()),
        TyArg::Const(_) => None,
    })
}

pub(crate) fn is_capitalized_type_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

pub(crate) fn list_element_ty_from_record_fields(
    fields: &[(Intern<String>, Box<Ty>)],
) -> Option<Ty> {
    let (_, pointer) = fields.iter().find(|(n, _)| n.as_str() == "pointer")?;
    if let Some(pointee) = pointer.pointee_ty() {
        return Some(pointee.clone());
    }
    match pointer.as_ref() {
        Ty::Opaque(name) if is_capitalized_type_name(name.as_str()) => {
            Some(pointer.as_ref().clone())
        }
        Ty::Record { name, .. }
            if is_capitalized_type_name(name.as_str()) && name.as_str() != "List" =>
        {
            Some(pointer.as_ref().clone())
        }
        _ => None,
    }
}

/// Format a `Ty` for hover display.
pub fn format_ty_for_hover(ty: &Ty) -> String {
    match ty {
        Ty::Named { .. } => ty.format_for_hover(),
        Ty::AnonymousInteger { .. } => ty.format_for_hover(),
        Ty::ResultFamily { .. } => ty.format_for_hover(),
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
