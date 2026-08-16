//! Compile-time trait dispatch: `Reflectable`, auto trait defaults, and provided trait overrides.

use std::collections::{HashMap, HashSet};

use internment::Intern;

use crate::analysis::ConstEnv;
use crate::reflect::{
    ReflectionRecursionState, ReflectionShapeConstructors, ReflectionTag,
    const_value_for_provided_trait_field, reflected_ty_to_const_value_with_schema,
};
use crate::ty::Ty;
use crate::typed::{TagId, TypedFileAst, TypedTag};
use ast::ConstValue;
use ast::FileAst;
use ast::declare::ProvidedTrait;
use ast::declare::ReflectionRole;
use ast::expr::BindValue;
use ast::span::SpanId;
use ast::ty::TypeId;

pub const REFLECTABLE_TRAIT: &str = "Reflectable";
pub const REFLECTABLE_SHAPE_FIELD: &str = "shape";
pub const RESERVED_TRAITS: &[&str] = &[REFLECTABLE_TRAIT];

const REFLECTION_ROLES: &[ReflectionRole] = &[
    ReflectionRole::Type,
    ReflectionRole::TypeNamed,
    ReflectionRole::TypeAnonymous,
    ReflectionRole::Bits,
    ReflectionRole::Address,
    ReflectionRole::Product,
    ReflectionRole::Sum,
    ReflectionRole::Array,
    ReflectionRole::Integer,
    ReflectionRole::Signed,
    ReflectionRole::Unsigned,
    ReflectionRole::Record,
    ReflectionRole::Union,
    ReflectionRole::Tuple,
    ReflectionRole::Unit,
    ReflectionRole::RawPointer,
    ReflectionRole::SafeReference,
    ReflectionRole::Observe,
    ReflectionRole::Mutate,
    ReflectionRole::TypeArgument,
    ReflectionRole::ConstArgument,
    ReflectionRole::DeclarationKey,
    ReflectionRole::RecursiveReference,
    ReflectionRole::Opaque,
    ReflectionRole::Unsupported,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReflectionRoleBinding {
    declaration_name: Intern<String>,
    declaration_span: SpanId,
    declaration_ids: Vec<TypeId>,
    declaration_keys: Vec<ast::ty::DeclarationKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReflectionPayloadShape {
    HasFields { len: usize },
    Alias,
}

fn reflection_role_payload_shape(role: ReflectionRole) -> Option<ReflectionPayloadShape> {
    match role {
        ReflectionRole::Type => None,
        ReflectionRole::TypeNamed => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::TypeAnonymous => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::Bits => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Address => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Product => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Sum => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Array => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Integer => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Signed => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::Unsigned => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::Record => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Union => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Tuple => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::Unit => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::RawPointer => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::SafeReference => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Observe => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::Mutate => Some(ReflectionPayloadShape::Alias),
        ReflectionRole::TypeArgument => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::ConstArgument => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::DeclarationKey => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::RecursiveReference => Some(ReflectionPayloadShape::HasFields { len: 2 }),
        ReflectionRole::Opaque => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::Unsupported => Some(ReflectionPayloadShape::HasFields { len: 1 }),
        ReflectionRole::TypeFormer | ReflectionRole::Trait | ReflectionRole::Shape => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionPayloadCheck {
    MissingDeclaration,
    ShapeMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionContractIssue {
    MissingRole {
        role: ReflectionRole,
        contract_name: Intern<String>,
        contract_span: SpanId,
    },
    DuplicateRole {
        role: ReflectionRole,
        declarations: Vec<(Intern<String>, SpanId)>,
    },
    DuplicateReflectionTrait {
        declarations: Vec<(Intern<String>, SpanId)>,
    },
    MalformedRole {
        role: ReflectionRole,
        declaration: Intern<String>,
        declaration_span: SpanId,
        check: ReflectionPayloadCheck,
    },
    RolesWithoutReflectionTrait {
        declarations: Vec<(Intern<String>, SpanId)>,
    },
    ReflectionContractIsInvalid,
}

impl ReflectionContractIssue {
    pub fn diagnostic(&self) -> diagnostic::Diagnostic {
        use diagnostic::Diagnostic;
        match self {
            Self::MissingRole {
                role,
                contract_name,
                ..
            } => Diagnostic::new(
                "type-missing-reflection-role",
                format!(
                    "missing reflection role `{}` in contract `{}`",
                    role.as_str(),
                    contract_name.as_str()
                ),
            )
            .with_arg("role", role.as_str().to_string())
            .with_arg("contract", contract_name.as_str().to_string()),
            Self::DuplicateRole {
                role,
                declarations,
            } => Diagnostic::new(
                "type-duplicate-reflection-role",
                format!(
                    "reflection role `{}` is declared {} times",
                    role.as_str(),
                    declarations.len()
                ),
            )
            .with_arg("role", role.as_str().to_string())
            .with_arg(
                "declarations",
                declarations
                    .iter()
                    .map(|(name, _)| name.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::DuplicateReflectionTrait { declarations } => Diagnostic::new(
                "type-duplicate-reflection-trait",
                "reflection trait is declared more than once",
            )
            .with_arg(
                "declarations",
                declarations
                    .iter()
                    .map(|(name, _)| name.as_str().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Self::MalformedRole {
                role,
                declaration,
                check,
                ..
            } => match check {
                ReflectionPayloadCheck::MissingDeclaration => Diagnostic::new(
                    "type-missing-reflection-role-declaration",
                    "the reflection role declaration is malformed and cannot be resolved",
                )
                .with_arg("role", role.as_str().to_string())
                .with_arg("declaration", declaration.as_str().to_string()),
                ReflectionPayloadCheck::ShapeMismatch { expected, actual } => Diagnostic::new(
                    "type-invalid-reflection-role-payload",
                    format!(
                        "reflection role `{}` on `{}` must have payload `{expected}`, found `{actual}`",
                        role.as_str(),
                        declaration.as_str()
                    ),
                )
                .with_arg("role", role.as_str().to_string())
                .with_arg("declaration", declaration.as_str().to_string())
                .with_arg("expected", expected.clone())
                .with_arg("actual", actual.clone()),
            },
            Self::RolesWithoutReflectionTrait { .. } => Diagnostic::new(
                "type-reflection-roles-without-trait",
                "reflection roles are declared without a reflection trait",
            ),
            Self::ReflectionContractIsInvalid => Diagnostic::new(
                "type-invalid-reflection-contract",
                "reflection contract declaration is invalid",
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionContract {
    pub trait_name: Intern<String>,
    pub trait_id: TypeId,
    pub trait_key: Option<ast::ty::DeclarationKey>,
    pub(crate) roles: HashMap<ReflectionRole, ReflectionRoleBinding>,
}

#[derive(Debug, Clone)]
pub struct ReflectionContractDiscovered {
    pub contract: Option<ReflectionContract>,
    pub issues: Vec<ReflectionContractIssue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VisitedTraitKey {
    Id(TypeId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultTypeFormerContract {
    pub declaration: TypeId,
    pub declaration_name: Intern<String>,
    pub declaration_type: crate::ty::Ty,
    pub fixed_array_contract: Option<ast::FixedArrayContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultTypeFormerResolutionError {
    Missing,
    Ambiguous,
    Invalid,
}

#[derive(Debug, Clone)]
pub struct CompileTimeTraitRegistry {
    pub imported_traits: HashSet<Intern<String>>,
    /// Shared package-level eval AST; cloning the registry only bumps the Arc refcount
    /// instead of duplicating ~32 files' worth of definitions per `TypedFileAst`.
    pub eval_ast: std::sync::Arc<FileAst>,
}

impl Default for CompileTimeTraitRegistry {
    fn default() -> Self {
        Self {
            imported_traits: HashSet::new(),
            eval_ast: std::sync::Arc::new(FileAst::empty_for_tests()),
        }
    }
}

impl CompileTimeTraitRegistry {
    pub fn resolve_default_type_former(
        &self,
        typed: &TypedFileAst,
        role: ast::DefaultTypeFormerRole,
    ) -> Result<DefaultTypeFormerContract, DefaultTypeFormerResolutionError> {
        let mut valid: Vec<DefaultTypeFormerContract> = Vec::new();
        let mut invalid: Vec<DefaultTypeFormerContract> = Vec::new();

        for (name, declaration) in &self.eval_ast.tags {
            if declaration.attributes.default_type_former != Some(role) {
                continue;
            }
            if !self.trait_in_scope(name.as_str(), typed) {
                continue;
            }

            let Some(ty) = typed.tag_types.get(&TagId(*name)) else {
                continue;
            };
            let Some(declaration_id) = ty.type_id() else {
                continue;
            };

            let fixed_array_contract =
                declaration.attributes.fixed_array.as_ref().map(|contract| {
                    ast::FixedArrayContract {
                        element_parameter: contract.element_parameter,
                        length_parameter: contract.length_parameter,
                        span: contract.span,
                    }
                });

            let candidate = DefaultTypeFormerContract {
                declaration: declaration_id,
                declaration_name: *name,
                declaration_type: ty.clone(),
                fixed_array_contract,
            };

            if !validate_default_type_former_contract(
                typed,
                declaration,
                role,
                &candidate.declaration_type,
            ) {
                invalid.push(candidate);
                continue;
            }
            valid.push(candidate);
        }

        if let [single] = valid.as_slice() {
            return Ok(single.clone());
        }
        if !valid.is_empty() {
            return Err(DefaultTypeFormerResolutionError::Ambiguous);
        }
        if !invalid.is_empty() {
            return Err(DefaultTypeFormerResolutionError::Invalid);
        }
        Err(DefaultTypeFormerResolutionError::Missing)
    }

    pub fn from_file_ast(file: &FileAst) -> Self {
        Self::from_parse_ast(file, std::sync::Arc::new(file.clone()))
    }

    pub fn from_parse_ast(file_ast: &FileAst, eval_ast: std::sync::Arc<FileAst>) -> Self {
        Self {
            imported_traits: file_ast.imported_trait_names(),
            eval_ast,
        }
    }

    pub fn trait_in_scope(&self, trait_name: &str, typed: &TypedFileAst) -> bool {
        if self.is_reflection_trait_name(trait_name, typed) {
            return true;
        }
        if self
            .eval_ast
            .tags
            .get(&Intern::from_ref(trait_name))
            .is_some_and(|declaration| {
                declaration
                    .attributes
                    .reflection_roles
                    .contains(&ReflectionRole::Trait)
            })
            && self.reflection_contract(typed).is_none()
        {
            return false;
        }
        let requested = canonical_trait_ids(trait_name, typed, &self.eval_ast);
        if requested.is_empty() {
            return self
                .imported_traits
                .iter()
                .any(|t| t.as_str() == trait_name)
                || self
                    .eval_ast
                    .tags
                    .contains_key(&Intern::from_ref(trait_name));
        }

        if self.imported_traits.iter().any(|imported| {
            ids_overlap(
                &requested,
                &canonical_trait_ids(imported.as_str(), typed, &self.eval_ast),
            )
        }) {
            return true;
        }

        self.eval_ast.tags.values().any(|decl| {
            ids_overlap(
                &requested,
                &canonical_trait_ids(decl.name.as_str(), typed, &self.eval_ast),
            )
        })
    }

    pub fn is_reflection_trait_name(&self, trait_name: &str, typed: &TypedFileAst) -> bool {
        let Some(contract) = self.reflection_contract(typed) else {
            return false;
        };
        if contract.trait_name.as_str() == trait_name {
            return true;
        }
        if let Some(key) = &contract.trait_key {
            let keys = declaration_keys_for_name(trait_name, typed, &self.eval_ast);
            return keys.iter().any(|candidate| candidate == key);
        }
        if canonical_trait_ids(trait_name, typed, &self.eval_ast).contains(&contract.trait_id) {
            return true;
        }
        false
    }

    pub fn reflection_contract(&self, typed: &TypedFileAst) -> Option<ReflectionContract> {
        self.reflection_contract_with_issues(typed).contract
    }

    pub fn reflection_contract_with_issues(
        &self,
        typed: &TypedFileAst,
    ) -> ReflectionContractDiscovered {
        let mut role_bindings: HashMap<ReflectionRole, Vec<ReflectionRoleBinding>> = HashMap::new();
        let mut trait_bindings: Vec<ReflectionRoleBinding> = Vec::new();
        let mut roles_declared = Vec::new();

        for (name, declaration) in &self.eval_ast.tags {
            let tag_id = TagId(*name);
            if !typed.tags.contains_key(&tag_id) && !typed.tag_types.contains_key(&tag_id) {
                continue;
            }

            let declaration_ids = declaration_ids_for_name(name.as_str(), typed, &self.eval_ast);
            let declaration_keys = declaration_ids
                .iter()
                .filter_map(|id| typed.type_registry.declaration(*id))
                .map(|declaration| declaration.key.clone())
                .collect::<Vec<_>>();
            let binding = ReflectionRoleBinding {
                declaration_name: *name,
                declaration_span: declaration.name_span,
                declaration_ids: declaration_ids.clone(),
                declaration_keys: declaration_keys.clone(),
            };
            if declaration
                .attributes
                .reflection_roles
                .contains(&ReflectionRole::Trait)
            {
                trait_bindings.push(binding.clone());
            }
            for role in &declaration.attributes.reflection_roles {
                role_bindings
                    .entry(*role)
                    .or_default()
                    .push(binding.clone());
                if *role != ReflectionRole::Trait {
                    roles_declared.push((role, binding.clone()));
                }
            }
        }

        let mut issues = Vec::new();
        if trait_bindings.len() > 1 {
            issues.push(ReflectionContractIssue::DuplicateReflectionTrait {
                declarations: trait_bindings
                    .iter()
                    .map(|binding| (binding.declaration_name, binding.declaration_span))
                    .collect(),
            });
            return ReflectionContractDiscovered {
                contract: None,
                issues,
            };
        }

        let Some(trait_binding) = trait_bindings.into_iter().next() else {
            if !roles_declared.is_empty() {
                let declarations = roles_declared
                    .into_iter()
                    .map(|(_, binding)| (binding.declaration_name, binding.declaration_span))
                    .collect();
                issues.push(ReflectionContractIssue::RolesWithoutReflectionTrait { declarations });
            }
            return ReflectionContractDiscovered {
                contract: None,
                issues,
            };
        };

        let trait_id = match trait_binding.declaration_ids.as_slice() {
            [single] => *single,
            [] => TypeId {
                file: typed.file_id.0,
                name: trait_binding.declaration_name,
            },
            _ => {
                issues.push(ReflectionContractIssue::ReflectionContractIsInvalid);
                return ReflectionContractDiscovered {
                    contract: None,
                    issues,
                };
            }
        };
        let reflection_contract_ids = reflection_trait_ids(self, typed);
        if reflection_contract_ids.len() != 1 || reflection_contract_ids[0] != trait_id {
            issues.push(ReflectionContractIssue::ReflectionContractIsInvalid);
            return ReflectionContractDiscovered {
                contract: None,
                issues,
            };
        }

        let mut roles = HashMap::new();
        for role in REFLECTION_ROLES {
            let declarations = role_bindings.remove(role).unwrap_or_default();
            if declarations.is_empty() {
                issues.push(ReflectionContractIssue::MissingRole {
                    role: *role,
                    contract_name: trait_binding.declaration_name,
                    contract_span: trait_binding.declaration_span,
                });
                continue;
            }
            if declarations.len() > 1 {
                issues.push(ReflectionContractIssue::DuplicateRole {
                    role: *role,
                    declarations: declarations
                        .iter()
                        .map(|binding| (binding.declaration_name, binding.declaration_span))
                        .collect(),
                });
                continue;
            }
            let declaration = declarations[0].clone();
            if let Some(check) = validate_reflection_role_payload(
                *role,
                &declaration.declaration_name,
                &declaration,
                typed,
            ) {
                issues.push(ReflectionContractIssue::MalformedRole {
                    role: *role,
                    declaration: declaration.declaration_name,
                    declaration_span: declaration.declaration_span,
                    check,
                });
                continue;
            }
            roles.insert(*role, declaration);
        }

        let has_blocking_issue = issues.iter().any(|issue| {
            !matches!(
                issue,
                ReflectionContractIssue::MissingRole { .. }
                    | ReflectionContractIssue::RolesWithoutReflectionTrait { .. }
            )
        });
        if has_blocking_issue {
            return ReflectionContractDiscovered {
                contract: None,
                issues,
            };
        }

        ReflectionContractDiscovered {
            contract: Some(ReflectionContract {
                trait_name: trait_binding.declaration_name,
                trait_id,
                trait_key: trait_binding.declaration_keys.first().cloned(),
                roles,
            }),
            issues,
        }
    }

    pub fn is_reserved_trait_name(&self, trait_name: &str, typed: &TypedFileAst) -> bool {
        self.is_reflection_trait_name(trait_name, typed)
    }
}

/// Synthetic `Reflectable` provided trait for a resolved type.
pub fn synthesize_reflectable_trait(
    ty: &Ty,
    typed: &TypedFileAst,
    trait_registry: &CompileTimeTraitRegistry,
    type_registry: &crate::TypeRegistry,
) -> Option<ProvidedTrait> {
    let contract = trait_registry.reflection_contract(typed)?;
    let schema = reflection_shape_schema(typed, trait_registry)?;
    let mut seen = ReflectionRecursionState::default();
    let cv = reflected_ty_to_const_value_with_schema(ty, &mut seen, Some(type_registry), &schema);
    Some(ProvidedTrait {
        trait_name: contract.trait_name,
        trait_name_span: ast::span::SpanId::INVALID,
        fields: vec![(
            Intern::from_ref(REFLECTABLE_SHAPE_FIELD),
            const_value_for_provided_trait_field(cv, ast::span::SpanId::INVALID),
        )],
    })
}

fn reflected_ty_to_reflection_shape(
    ty: &Ty,
    registry: &crate::TypeRegistry,
    schema: &ReflectionShapeConstructors,
    seen: &mut ReflectionRecursionState,
) -> ConstValue {
    reflected_ty_to_const_value_with_schema(ty, seen, Some(registry), schema)
}

fn reflection_shape_schema(
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
) -> Option<ReflectionShapeConstructors> {
    let contract = registry.reflection_contract(typed)?;
    let role = |role| reflection_role_tag(&contract.roles, role);
    Some(ReflectionShapeConstructors {
        signed: role(ReflectionRole::Signed)?,
        unsigned: role(ReflectionRole::Unsigned)?,
        raw_pointer: role(ReflectionRole::RawPointer)?,
        safe_reference: role(ReflectionRole::SafeReference)?,
        record: role(ReflectionRole::Record)?,
        union: role(ReflectionRole::Union)?,
        tuple: role(ReflectionRole::Tuple)?,
        array: role(ReflectionRole::Array)?,
        integer: role(ReflectionRole::Integer)?,
        unit: role(ReflectionRole::Unit)?,
        opaque: role(ReflectionRole::Opaque)?,
        unsupported: role(ReflectionRole::Unsupported)?,
        recursive_reference: role(ReflectionRole::RecursiveReference)?,
        observe_permission: role(ReflectionRole::Observe)?,
        mutate_permission: role(ReflectionRole::Mutate)?,
    })
}

fn reflection_role_tag(
    roles: &std::collections::HashMap<ast::declare::ReflectionRole, ReflectionRoleBinding>,
    role: ast::declare::ReflectionRole,
) -> Option<ReflectionTag> {
    let binding = roles.get(&role)?;
    let qual_path = binding
        .declaration_keys
        .first()
        .map(declaration_key_to_qual_path);
    Some(ReflectionTag {
        name: binding.declaration_name,
        qual_path: qual_path.map(|s| s.to_string()),
    })
}

fn declaration_key_to_qual_path(key: &ast::ty::DeclarationKey) -> String {
    let mut segments = Vec::with_capacity(1 + key.module.len());
    segments.push(key.package.name.as_str().to_string());
    segments.extend(key.module.iter().map(|part| part.as_str().to_string()));
    segments.join(".")
}

/// Look up a trait field value for a nominal type by name.
pub fn trait_field_for_type_name(
    trait_name: &str,
    field_name: &str,
    type_name: Intern<String>,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
    file: Option<&FileAst>,
) -> Option<ConstValue> {
    let mut seen = ReflectionRecursionState::default();
    trait_field_for_type_name_with_seen(
        trait_name, field_name, type_name, typed, registry, file, &mut seen,
    )
}

fn trait_field_for_type_name_with_seen(
    trait_name: &str,
    field_name: &str,
    type_name: Intern<String>,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
    file: Option<&FileAst>,
    seen: &mut ReflectionRecursionState,
) -> Option<ConstValue> {
    let tag_id = TagId(type_name);
    let requested_trait_ids = canonical_trait_ids(trait_name, typed, typed.eval_ast.as_ref());
    if !registry.trait_in_scope(trait_name, typed) {
        return None;
    }
    if registry.is_reflection_trait_name(trait_name, typed) && field_name == REFLECTABLE_SHAPE_FIELD
    {
        let schema = reflection_shape_schema(typed, registry)?;
        if let Some(tag) = typed.tags.get(&tag_id) {
            return reflectable_shape_from_tag(tag, typed, registry, &typed.type_registry, seen);
        }
        if let Some(ty) = typed.tag_types.get(&tag_id) {
            return Some(reflected_ty_to_reflection_shape(
                ty,
                &typed.type_registry,
                &schema,
                seen,
            ));
        }
        return None;
    }

    if let Some(tag) = typed.tags.get(&tag_id) {
        let eval_ast = file.unwrap_or(registry.eval_ast.as_ref());
        if let Some(cv) = provided_trait_field(
            tag,
            trait_name,
            field_name,
            &requested_trait_ids,
            typed,
            eval_ast,
        ) {
            return Some(cv);
        }
    }

    None
}

/// Look up a trait field for any resolved type, including auto trait defaults.
pub fn trait_field_for_ty(
    trait_name: &str,
    field_name: &str,
    ty: &Ty,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
) -> Option<ConstValue> {
    let mut seen = ReflectionRecursionState::default();
    trait_field_for_ty_with_seen(trait_name, field_name, ty, typed, registry, &mut seen)
}

fn trait_field_for_ty_with_seen(
    trait_name: &str,
    field_name: &str,
    ty: &Ty,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
    seen: &mut ReflectionRecursionState,
) -> Option<ConstValue> {
    if !registry.trait_in_scope(trait_name, typed) {
        return None;
    }
    if registry.is_reflection_trait_name(trait_name, typed) && field_name == REFLECTABLE_SHAPE_FIELD
    {
        let schema = reflection_shape_schema(typed, registry)?;
        return Some(reflected_ty_to_reflection_shape(
            ty,
            &typed.type_registry,
            &schema,
            seen,
        ));
    }
    if let Some(cv) =
        provided_trait_field_for_matching_ty(trait_name, field_name, ty, typed, registry)
    {
        return Some(cv);
    }
    if let Some(type_name) = ty.type_name()
        && let Some(cv) = trait_field_for_type_name_with_seen(
            trait_name,
            field_name,
            *type_name,
            typed,
            registry,
            Some(registry.eval_ast.as_ref()),
            seen,
        )
    {
        return Some(cv);
    }
    None
}

fn provided_trait_field_for_matching_ty(
    trait_name: &str,
    field_name: &str,
    ty: &Ty,
    typed: &TypedFileAst,
    registry: &CompileTimeTraitRegistry,
) -> Option<ConstValue> {
    let requested_trait_ids = canonical_trait_ids(trait_name, typed, typed.eval_ast.as_ref());
    let id = ty.type_id()?;
    if id.file != typed.file_id.0 {
        return None;
    }
    let tag = typed.tags.get(&TagId(id.name))?;
    provided_trait_field(
        tag,
        trait_name,
        field_name,
        &requested_trait_ids,
        typed,
        registry.eval_ast.as_ref(),
    )
}

fn reflectable_shape_from_tag(
    tag: &TypedTag,
    typed: &TypedFileAst,
    trait_registry: &CompileTimeTraitRegistry,
    registry: &crate::TypeRegistry,
    seen: &mut ReflectionRecursionState,
) -> Option<ConstValue> {
    let schema = reflection_shape_schema(typed, trait_registry)?;
    for pt in &tag.provided_traits {
        if trait_registry.is_reflection_trait_name(pt.trait_name.as_str(), typed) {
            for (name, expr) in &pt.fields {
                if name.as_str() == REFLECTABLE_SHAPE_FIELD {
                    return expr.const_value.clone();
                }
            }
        }
    }
    Some(reflected_ty_to_reflection_shape(
        &tag.resolved_ty,
        registry,
        &schema,
        seen,
    ))
}

fn validate_default_type_former_contract(
    typed: &TypedFileAst,
    declaration: &ast::Declare,
    role: ast::DefaultTypeFormerRole,
    declaration_type: &crate::ty::Ty,
) -> bool {
    match role {
        ast::DefaultTypeFormerRole::RawPointer => {
            let Some(element_name) = declaration
                .params
                .as_ref()
                .and_then(|params| params.keys().next())
            else {
                return false;
            };
            if declaration
                .params
                .as_ref()
                .is_none_or(|params| params.len() != 1)
            {
                return false;
            }
            if !crate::representation::Repr::derive(declaration_type, Some(&typed.type_registry))
                .is_ok_and(|repr| matches!(repr, crate::representation::Repr::Address { .. }))
            {
                return false;
            }
            typed
                .type_registry
                .resolved_definition_for_type(declaration_type)
                .pointee_ty().is_some_and(
                |pointee| matches!(pointee, crate::ty::Ty::Opaque(name) if name == element_name),
            )
        }
        ast::DefaultTypeFormerRole::FixedArray => typed
            .type_registry
            .declaration_for_type(declaration_type)
            .is_some_and(|declaration| declaration.fixed_array.is_some()),
    }
}

fn provided_trait_field(
    tag: &TypedTag,
    trait_name: &str,
    field_name: &str,
    requested_trait_ids: &[TypeId],
    typed: &TypedFileAst,
    eval_ast: &FileAst,
) -> Option<ConstValue> {
    let mut visited = HashSet::<VisitedTraitKey>::new();
    provided_trait_field_with_traits(
        &tag.provided_traits,
        trait_name,
        field_name,
        requested_trait_ids,
        typed,
        eval_ast,
        &mut visited,
    )
}

fn provided_trait_field_with_traits(
    provided_traits: &[ProvidedTrait],
    trait_name: &str,
    field_name: &str,
    requested_trait_ids: &[TypeId],
    typed: &TypedFileAst,
    eval_ast: &FileAst,
    visited: &mut HashSet<VisitedTraitKey>,
) -> Option<ConstValue> {
    for pt in provided_traits {
        let trait_ids = canonical_trait_ids(pt.trait_name.as_str(), typed, eval_ast);
        let should_visit = if trait_ids.is_empty() {
            continue;
        } else {
            let mut saw_new = false;
            for id in trait_ids {
                saw_new |= visited.insert(VisitedTraitKey::Id(id));
            }
            saw_new
        };
        if !should_visit {
            continue;
        }
        let matches_requested = if requested_trait_ids.is_empty() {
            pt.trait_name.as_str() == trait_name
        } else {
            ids_overlap(
                requested_trait_ids,
                &canonical_trait_ids(pt.trait_name.as_str(), typed, eval_ast),
            )
        };
        if matches_requested {
            for (name, expr) in &pt.fields {
                if name.as_str() == field_name {
                    return expr.const_value.clone().or_else(|| {
                        let empty_env = ConstEnv::default();
                        let evaluator =
                            crate::analysis::CompTimeEvaluator::new(&empty_env, eval_ast);
                        evaluator.eval(&expr.value)
                    });
                }
            }
            if let Some(cv) = trait_member_from_bind(pt.trait_name.as_str(), field_name, eval_ast) {
                return Some(cv);
            }
        }
        let nested_provided = if let Some(nested) = typed.tags.get(&TagId(pt.trait_name)) {
            Some(&nested.provided_traits as &[ProvidedTrait])
        } else {
            eval_ast
                .tags
                .get(&pt.trait_name)
                .map(|declaration| declaration.provided_traits.as_slice())
        };
        if let Some(nested_provided) = nested_provided
            && let Some(cv) = provided_trait_field_with_traits(
                nested_provided,
                trait_name,
                field_name,
                requested_trait_ids,
                typed,
                eval_ast,
                visited,
            )
        {
            return Some(cv);
        }
    }

    None
}

fn trait_member_from_bind(
    trait_name: &str,
    field_name: &str,
    eval_ast: &FileAst,
) -> Option<ConstValue> {
    let bind_name = Intern::new(format!("{}.{}", trait_name, field_name));
    let bind = eval_ast.defs.get(&bind_name)?;
    if bind.params.is_some() {
        return None;
    }
    match &bind.value {
        BindValue::Expr(expr) => {
            let const_bindings = ConstEnv::default();
            let evaluator = crate::analysis::CompTimeEvaluator::new(&const_bindings, eval_ast);
            evaluator.eval(&expr.value)
        }
        _ => None,
    }
}

fn canonical_trait_ids(name: &str, typed: &TypedFileAst, eval_ast: &ast::FileAst) -> Vec<TypeId> {
    let mut seen = HashSet::new();
    canonical_trait_ids_inner(name, typed, eval_ast, &mut seen)
}

fn canonical_trait_ids_inner(
    name: &str,
    typed: &TypedFileAst,
    eval_ast: &ast::FileAst,
    seen: &mut HashSet<Intern<String>>,
) -> Vec<TypeId> {
    let name = Intern::from_ref(name);
    if !seen.insert(name) {
        return Vec::new();
    }

    let mut ids = Vec::new();
    if let Some(ty) = typed.tag_types.get(&TagId(name)) {
        if let Some(id) = canonical_target_type_id(ty, typed, &mut HashSet::new()) {
            ids.push(id);
        } else if let Some(id) = ty.type_id() {
            ids.push(id);
        }
    }
    for id in typed
        .type_registry
        .declaration_ids_by_display_name(name.as_str())
    {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    if eval_ast
        .tags
        .get(&name)
        .is_some_and(|declaration| matches!(declaration.value, ast::DeclareValue::Alias(_)))
    {
        for alias_target in alias_target_names(name, eval_ast) {
            if alias_target == name {
                continue;
            }
            for id in canonical_trait_ids_inner(alias_target.as_str(), typed, eval_ast, seen) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }

    if ids.is_empty() && typed.tags.contains_key(&TagId(name)) {
        ids.push(TypeId {
            file: typed.file_id.0,
            name,
        });
    }

    ids
}

fn canonical_target_type_id(
    ty: &Ty,
    typed: &TypedFileAst,
    seen: &mut HashSet<TypeId>,
) -> Option<TypeId> {
    let instance = ty.named_instance()?;
    if !seen.insert(instance.declaration) {
        return None;
    }
    if typed
        .type_registry
        .declaration(instance.declaration)
        .is_some()
    {
        return Some(instance.declaration);
    }
    None
}

fn alias_target_names(name: Intern<String>, eval_ast: &ast::FileAst) -> Vec<Intern<String>> {
    let declaration = match eval_ast.tags.get(&name) {
        Some(decl) => decl,
        None => return Vec::new(),
    };

    match &declaration.value {
        ast::DeclareValue::Alias(alias) => match alias.value() {
            ast::Expr::AnonymousTag(target) => vec![*target],
            ast::Expr::TagCall(tc) if tc.args.is_empty() && tc.qual_path.is_none() => {
                vec![tc.name]
            }
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn declaration_ids_for_name(
    name: &str,
    typed: &TypedFileAst,
    eval_ast: &ast::FileAst,
) -> Vec<TypeId> {
    canonical_trait_ids(name, typed, eval_ast)
}

fn declaration_keys_for_name(
    name: &str,
    typed: &TypedFileAst,
    eval_ast: &ast::FileAst,
) -> Vec<ast::ty::DeclarationKey> {
    let mut keys = Vec::new();
    for id in declaration_ids_for_name(name, typed, eval_ast) {
        if let Some(declaration) = typed.type_registry.declaration(id)
            && !keys.contains(&declaration.key)
        {
            keys.push(declaration.key.clone());
        }
    }
    keys
}

fn reflection_trait_ids(registry: &CompileTimeTraitRegistry, typed: &TypedFileAst) -> Vec<TypeId> {
    let mut ids = Vec::new();
    for declaration in registry.eval_ast.tags.values() {
        if !declaration
            .attributes
            .reflection_roles
            .contains(&ReflectionRole::Trait)
        {
            continue;
        }
        for id in canonical_trait_ids(declaration.name.as_str(), typed, registry.eval_ast.as_ref())
        {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn validate_reflection_role_payload(
    role: ReflectionRole,
    declaration_name: &Intern<String>,
    binding: &ReflectionRoleBinding,
    typed: &TypedFileAst,
) -> Option<ReflectionPayloadCheck> {
    let expected = reflection_role_payload_shape(role)?;
    let declaration_ty = typed.tag_types.get(&TagId(*declaration_name)).or_else(|| {
        binding
            .declaration_ids
            .iter()
            .find_map(|id| typed.tag_types.get(&TagId(id.name)))
    })?;
    let normalized_decl_ty = typed
        .type_registry
        .resolved_definition_for_type(declaration_ty);
    if matches!(expected, ReflectionPayloadShape::Alias) {
        return None;
    }
    if let ReflectionPayloadShape::HasFields { .. } = expected
        && (matches!(&normalized_decl_ty, crate::ty::Ty::Record { .. })
            || matches!(&normalized_decl_ty, crate::ty::Ty::Opaque(_)))
    {
        return None;
    }

    let actual = match &normalized_decl_ty {
        crate::ty::Ty::Record { fields, .. } => {
            format!("record({} fields)", fields.len())
        }
        crate::ty::Ty::Union { variants, .. } => {
            format!("union({} variants)", variants.len())
        }
        other => format!("{other:?}(no payload)"),
    };
    let expected = match expected {
        ReflectionPayloadShape::HasFields { len } => format!("record({} fields)", len),
        ReflectionPayloadShape::Alias => "alias".to_string(),
    };
    Some(ReflectionPayloadCheck::ShapeMismatch { expected, actual })
}

fn ids_overlap(lhs: &[TypeId], rhs: &[TypeId]) -> bool {
    lhs.iter().any(|id| rhs.contains(id))
}

#[cfg(test)]
#[path = "../tests/compile_time_trait_tests.rs"]
mod tests;
