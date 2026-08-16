use crate::compile_time_trait::CompileTimeTraitRegistry;
use crate::{FileId, typed::TypedFileAst};
use ast::FileAst;
use flask::CompileTarget;
use interface::{
    DeclarationRef, Fingerprint, InterfaceDeclaration, InterfacePublication, PackageInstanceId,
    PublicInterface,
};
use std::collections::HashSet;
use std::sync::Arc;

use super::{PackageTransformOptions, TransformCtx, transform_package};

pub struct PackageTransformArtifacts {
    pub typed_asts: Vec<TypedFileAst>,
    pub compile_time_eval_ast: Arc<FileAst>,
    pub trait_registry: Option<CompileTimeTraitRegistry>,
    pub public_interface: PublicInterface,
    pub interface_publication: InterfacePublication,
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

    let mut declarations = Vec::new();
    let mut seen_declaration_keys = HashSet::new();
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
            if seen_declaration_keys.insert(declaration_key.clone()) {
                let declaration_bytes = encode_callable_surface(&declaration_key, bind);
                declarations.push(InterfaceDeclaration {
                    reference,
                    fingerprint: Fingerprint::from_bytes(&declaration_bytes),
                });
            }
        }
    }

    let target_profiles = target_profiles_for_compile_target(&options.compile_target);
    let subject = PackageInstanceId {
        package: package_ctx.package_instance.name.as_str().to_string(),
        version: package_ctx.package_instance.version.as_str().to_string(),
        source: format!("{:?}", package_ctx.package_instance.source),
        instance: package_ctx.package_instance.instance.as_str().to_string(),
    };
    let public_interface = if target_profiles.is_empty() {
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
    let has_fatal_flaws = typed_asts.iter().any(|typed| !typed.all_flaws().is_empty());
    let interface_publication =
        InterfacePublication::resolve(public_interface.clone(), has_fatal_flaws, None);

    PackageTransformArtifacts {
        typed_asts,
        compile_time_eval_ast,
        trait_registry,
        public_interface,
        interface_publication,
    }
}

fn encode_callable_surface(declaration_key: &str, bind: &crate::typed::TypedBind) -> Vec<u8> {
    let mut bytes = Vec::new();
    encode_string(&mut bytes, declaration_key);
    encode_u32(&mut bytes, bind.params.len());
    for (_, ty) in &bind.params {
        encode_ty(&mut bytes, ty);
    }
    encode_ty(&mut bytes, &bind.return_type);
    bytes
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
        ast::Ty::Tuple(fields) => fields
            .iter()
            .any(|field| ty_contains_private_identity(field, private_names, erasable_aliases)),
        ast::Ty::Array { elem, .. } => {
            ty_contains_private_identity(elem, private_names, erasable_aliases)
        }
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
