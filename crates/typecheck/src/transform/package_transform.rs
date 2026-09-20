use crate::compile_time_trait::CompileTimeTraitRegistry;
use crate::{FileId, typed::TypedFileAst};
use ast::FileAst;
use ast_format::type_expr::ExprFormatExt;
use flask::CompileTarget;
use interface::{
    DeclarationRef, Fingerprint, InterfaceDeclaration, InterfacePublication, PackageInstanceId,
    PublicInterface,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use super::{PackageTransformOptions, TransformCtx, transform_package};

pub struct PackageTransformArtifacts {
    pub typed_asts: Vec<TypedFileAst>,
    pub compile_time_eval_ast: Arc<FileAst>,
    pub trait_registry: Option<CompileTimeTraitRegistry>,
    pub public_interface: PublicInterface,
    pub interface_publication: InterfacePublication,
    pub implementation_fingerprint: Fingerprint,
}

pub fn transform_package_with_shared_context(
    file_asts: Vec<FileAst>,
    options: PackageTransformOptions,
) -> PackageTransformArtifacts {
    transform_package_with_shared_context_and_package(
        file_asts,
        options,
        ast::ty::PackageInstanceKey::workspace("anonymous", "0"),
    )
}

pub fn transform_package_with_shared_context_and_package(
    mut file_asts: Vec<FileAst>,
    options: PackageTransformOptions,
    package: ast::ty::PackageInstanceKey,
) -> PackageTransformArtifacts {
    let mut compile_time_eval_ast = FileAst::default();
    for ast in &file_asts {
        compile_time_eval_ast.merge_from(ast.clone());
    }

    let compile_time_eval_ast = Arc::new(compile_time_eval_ast);
    let trait_registry = if options.ide_package {
        None
    } else {
        Some(CompileTimeTraitRegistry::from_parse_ast(
            &compile_time_eval_ast,
            Arc::clone(&compile_time_eval_ast),
        ))
    };

    let package_ctx =
        TransformCtx::with_package_compile_time_arc(Arc::clone(&compile_time_eval_ast))
            .with_package_instance(package);

    let file_asts_with_ids: Vec<(FileAst, FileId)> = file_asts
        .drain(..)
        .enumerate()
        .map(|(i, ast)| (ast, FileId(i as u32)))
        .collect();

    let mut typed_asts = transform_package(&file_asts_with_ids, &package_ctx, options.clone());

    let erasable_private_aliases: HashSet<_> = compile_time_eval_ast
        .private_tags
        .iter()
        .filter(|name| {
            compile_time_eval_ast
                .tags
                .get(name)
                .is_some_and(|declaration| matches!(declaration.value, ast::DeclareValue::Alias(_)))
        })
        .copied()
        .collect();
    for typed in &mut typed_asts {
        let private_type_names: HashSet<_> = typed.private_tags.iter().map(|tag| tag.0).collect();
        let public_defs: Vec<_> = typed
            .defs
            .keys()
            .filter(|definition| !typed.private_defs.contains(definition))
            .copied()
            .collect();
        for definition in public_defs {
            let Some(bind) = typed.defs.get_mut(&definition) else {
                continue;
            };
            if matches!(
                bind.return_type,
                ast::Ty::ResultFamily {
                    owner: ast::ResultFamilyOwner::LocalCallable(_),
                    ..
                }
            ) && !bind
                .flaws
                .iter()
                .any(|flaw| flaw.code.slug() == "interface-local-result-family-escapes")
            {
                bind.flaws.push(
                    diagnostic::Diagnostic::new(
                        "interface-local-result-family-escapes",
                        "a session-local result family cannot appear in a public interface",
                    )
                    .at_span_id(bind.name_span, &typed.span_table),
                );
            }
            let exposes_private_identity = bind.params.iter().any(|(_, ty)| {
                ty_contains_private_identity(ty, &private_type_names, &erasable_private_aliases)
            }) || ty_contains_private_identity(
                &bind.return_type,
                &private_type_names,
                &erasable_private_aliases,
            );
            if exposes_private_identity
                && !bind
                    .flaws
                    .iter()
                    .any(|flaw| flaw.code.slug() == "interface-public-private-identity")
            {
                bind.flaws.push(
                    diagnostic::Diagnostic::new(
                        "interface-public-private-identity",
                        "a public declaration exposes a residual private nominal identity",
                    )
                    .at_span_id(bind.name_span, &typed.span_table),
                );
            }
        }
    }

    let mut surfaces: BTreeMap<DeclarationRef, BTreeSet<Vec<u8>>> = BTreeMap::new();
    let mut resolution_surfaces = BTreeSet::new();
    for typed in &typed_asts {
        let module_path = typed
            .semantic_origin
            .as_ref()
            .map(|origin| {
                origin
                    .module
                    .iter()
                    .map(|segment| segment.as_str())
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .unwrap_or_else(|| "".to_string());

        for (id, tag) in &typed.tags {
            if typed.private_tags.contains(id) {
                continue;
            }
            let declaration_path = format!("{module_path}::type::{}", id.0.as_str());
            let key = format!("subject:{module_path}:{declaration_path}");
            {
                let mut bytes = Vec::new();
                encode_string(&mut bytes, &key);
                encode_string(&mut bytes, &tag.declaration_text);
                encode_ty(&mut bytes, &tag.resolved_ty);
                for provided in &tag.provided_traits {
                    encode_string(&mut bytes, provided.trait_name.as_str());
                    for (name, value) in &provided.fields {
                        encode_string(&mut bytes, name.as_str());
                        encode_string(&mut bytes, &value.value.format_surface());
                    }
                }
                if let Some(bits) = &tag.attributes.bits {
                    encode_string(&mut bytes, &bits.value.format_surface());
                }
                encode_string(
                    &mut bytes,
                    &format!("{:?}", tag.attributes.reflection_roles),
                );
                if !tag.provided_traits.is_empty() {
                    resolution_surfaces.insert(bytes.clone());
                }
                surfaces
                    .entry(DeclarationRef::Subject {
                        module_path: module_path.clone(),
                        declaration_path,
                    })
                    .or_default()
                    .insert(bytes);
            }
        }

        for (_, bind) in typed
            .defs
            .iter()
            .filter(|(def_id, _)| !typed.private_defs.contains(def_id))
        {
            let declaration_path = format!("{module_path}::{}", bind.name.as_str());
            let declaration_key = format!("subject:{module_path}:{declaration_path}");
            let reference = DeclarationRef::Subject {
                module_path: module_path.clone(),
                declaration_path,
            };
            let bytes = encode_callable_surface(&declaration_key, bind);
            if bind.infix_symbol.is_some() {
                resolution_surfaces.insert(bytes.clone());
            }
            surfaces.entry(reference).or_default().insert(bytes);
        }
    }

    let declarations = surfaces
        .into_iter()
        .map(|(reference, candidates)| {
            let mut bytes = Vec::new();
            for candidate in candidates {
                bytes.extend_from_slice(&(candidate.len() as u64).to_le_bytes());
                bytes.extend(candidate);
            }
            InterfaceDeclaration {
                reference,
                fingerprint: Fingerprint::from_bytes(&bytes),
            }
        })
        .collect();

    let implementation_fingerprint =
        implementation_fingerprint(&typed_asts, &options.compile_target);
    let has_assembly = typed_asts.iter().any(|typed| {
        typed.exprs.kind.iter().any(|kind| {
            matches!(
                kind,
                crate::TypedExprKind::Asm {
                    state: crate::assembly::AssemblyState::Checked(_),
                    ..
                }
            )
        })
    });
    let target_profiles = target_profiles_for_compile_target(&options.compile_target);
    let subject = PackageInstanceId {
        package: package_ctx.package_instance.name.as_str().to_string(),
        version: package_ctx.package_instance.version.as_str().to_string(),
        source: format!("{:?}", package_ctx.package_instance.source),
        instance: package_ctx.package_instance.instance.as_str().to_string(),
    };
    let mut public_interface = if target_profiles.is_empty() {
        PublicInterface::from_subject_and_declarations(subject, declarations)
    } else {
        PublicInterface::with_targets_and_realizations(
            subject,
            declarations,
            target_profiles,
            Vec::new(),
            Vec::new(),
        )
    };
    let mut resolution_bytes = Vec::new();
    for candidate in resolution_surfaces {
        resolution_bytes.extend_from_slice(&(candidate.len() as u64).to_le_bytes());
        resolution_bytes.extend(candidate);
    }
    public_interface.resolution_fingerprint = Fingerprint::from_bytes(&resolution_bytes);

    if has_assembly {
        for realization in &mut public_interface.target_realizations {
            realization.semantic_surface_fingerprint = Fingerprint::from_domain(
                b"gin.interface.target-assembly-surface\0",
                interface::SCHEMA_VERSION,
                &[
                    realization.semantic_surface_fingerprint.0.as_slice(),
                    implementation_fingerprint.0.as_slice(),
                ]
                .concat(),
            );
            realization.public_closure_fingerprint = Fingerprint::from_domain(
                b"gin.interface.target-assembly-closure\0",
                interface::SCHEMA_VERSION,
                &[
                    realization.public_closure_fingerprint.0.as_slice(),
                    implementation_fingerprint.0.as_slice(),
                ]
                .concat(),
            );
        }
    }
    let has_fatal_flaws = typed_asts.iter().any(|typed| !typed.all_flaws().is_empty());
    let interface_publication =
        InterfacePublication::resolve(public_interface.clone(), has_fatal_flaws, None);

    PackageTransformArtifacts {
        typed_asts,
        compile_time_eval_ast,
        trait_registry,
        public_interface,
        interface_publication,
        implementation_fingerprint,
    }
}

fn encode_callable_surface(declaration_key: &str, bind: &crate::typed::TypedBind) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_string(&mut bytes, declaration_key);
    encode_string(&mut bytes, &format!("{:?}", bind.param_conventions));
    encode_string(&mut bytes, &format!("{:?}", bind.param_kinds));
    encode_string(&mut bytes, &format!("{:?}", bind.call_capability));
    encode_string(&mut bytes, &format!("{:?}", bind.definition_stage));
    encode_string(&mut bytes, &format!("{:?}", bind.param_requirements));
    encode_string(&mut bytes, &format!("{:?}", bind.param_refinements));
    encode_string(&mut bytes, &format!("{:?}", bind.param_groups));
    encode_string(
        &mut bytes,
        bind.infix_symbol.as_ref().map(|s| s.as_str()).unwrap_or(""),
    );
    encode_u32(&mut bytes, bind.params.len());
    for (name, ty) in &bind.params {
        encode_string(&mut bytes, name.as_str());
        encode_ty(&mut bytes, ty);
    }
    let signature = crate::typed::TypedCallableSignature::from(bind);
    encode_u32(&mut bytes, signature.results.len());
    for result in &signature.results {
        if let Some(name) = result.name {
            encode_string(&mut bytes, name.as_str());
        } else {
            encode_string(&mut bytes, "");
        }
        encode_ty(&mut bytes, &result.ty);
    }
    encode_u32(&mut bytes, bind.result_groups.len());
    for group in &bind.result_groups {
        let path = group
            .and_then(|group| bind.groups.get(group.0 as usize))
            .and_then(|group| group.path.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default();
        encode_string(&mut bytes, &path);
    }
    encode_ty(&mut bytes, &bind.return_type);
    bytes
}

fn implementation_fingerprint(typed_asts: &[TypedFileAst], target: &CompileTarget) -> Fingerprint {
    let mut entries = typed_asts
        .iter()
        .flat_map(|typed| {
            typed.exprs.kind.iter().filter_map(|kind| {
                let crate::TypedExprKind::Asm {
                    operands, state, ..
                } = kind
                else {
                    return None;
                };
                let crate::assembly::AssemblyState::Checked(assembly) = state else {
                    return None;
                };
                let names = operands
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                Some(format!("{}\0{}\0", assembly.canonical_form(), names))
            })
        })
        .collect::<Vec<_>>();
    entries.sort();
    let mut bytes = format!("target={target:?}\0").into_bytes();
    for entry in entries {
        encode_string(&mut bytes, &entry);
    }
    Fingerprint::from_domain(
        b"gin.implementation.assembly\0",
        interface::SCHEMA_VERSION,
        &bytes,
    )
}

fn encode_ty(bytes: &mut Vec<u8>, ty: &ast::Ty) {
    match ty {
        ast::Ty::Named { instance, .. } => {
            bytes.push(0);
            bytes.extend_from_slice(&instance.declaration.file.to_le_bytes());
            encode_string(bytes, instance.declaration.name.as_str());
            encode_u32(bytes, instance.arguments.len());
            for (parameter, argument) in &instance.arguments {
                encode_string(bytes, parameter.as_str());
                match argument {
                    ast::ty::TyArg::Type(ty) => {
                        bytes.push(0);
                        encode_ty(bytes, ty);
                    }
                    ast::ty::TyArg::Const(value) => {
                        bytes.push(1);
                        encode_string(bytes, &value.to_string());
                    }
                }
            }
        }
        _ => {
            bytes.push(1);
            encode_string(bytes, &ty.format_for_hover());
        }
    }
}

fn encode_string(bytes: &mut Vec<u8>, value: &str) {
    encode_u32(bytes, value.len());
    bytes.extend_from_slice(value.as_bytes());
}

fn encode_u32(bytes: &mut Vec<u8>, value: usize) {
    bytes.extend_from_slice(&u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes());
}

fn ty_contains_private_identity(
    ty: &ast::Ty,
    private_names: &HashSet<internment::Intern<String>>,
    erasable_aliases: &HashSet<internment::Intern<String>>,
) -> bool {
    match ty {
        ast::Ty::Named { instance, .. } => {
            private_names.contains(&instance.declaration.name)
                && !erasable_aliases.contains(&instance.declaration.name)
        }
        ast::Ty::Record { fields, .. } => fields
            .iter()
            .any(|(_, field)| ty_contains_private_identity(field, private_names, erasable_aliases)),
        ast::Ty::Sequence(fields) => fields
            .iter()
            .any(|field| ty_contains_private_identity(&field.ty, private_names, erasable_aliases)),
        ast::Ty::Array { elem, .. } => {
            ty_contains_private_identity(elem, private_names, erasable_aliases)
        }
        ast::Ty::UnresolvedSequence { element, .. } => element.as_deref().is_some_and(|element| {
            ty_contains_private_identity(element, private_names, erasable_aliases)
        }),
        ast::Ty::Ref { inner, .. }
        | ast::Ty::Ptr { inner }
        | ast::Ty::Address { pointee: inner, .. } => {
            ty_contains_private_identity(inner, private_names, erasable_aliases)
        }
        ast::Ty::Union { variants, .. } => variants.iter().any(|variant| {
            variant.fields.iter().any(|(_, field)| {
                ty_contains_private_identity(field, private_names, erasable_aliases)
            })
        }),
        ast::Ty::ResultFamily { .. } => false,
        ast::Ty::UnresolvedLiteral(_)
        | ast::Ty::AnonymousInteger { .. }
        | ast::Ty::Float { .. }
        | ast::Ty::Unit
        | ast::Ty::Opaque(_)
        | ast::Ty::Literal(_) => false,
    }
}

fn target_profiles_for_compile_target(
    compile_target: &CompileTarget,
) -> Vec<interface::TargetProfile> {
    let target = match compile_target {
        CompileTarget::Library => None,
        CompileTarget::Concrete(target) => Some(target.raw.clone()),
    };

    target
        .map(|target| vec![interface::TargetProfile { target }])
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../tests/transform_package_transform_tests.rs"]
mod tests;
