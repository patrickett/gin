use diagnostic::code::MemberCode;
use std::collections::HashSet;

use ast::Ty;
use diagnostic::Diagnostic;
use internment::Intern;

use crate::ty::reference_pointee_for_type;
use crate::typed::{ExprId, SequenceTraversal, TypedExprKind, TypedFileAst, TypedLoopKind};

fn origin_has_grant(
    origin: &ast::prelude::FileSemanticOrigin,
    package: &ast::ty::PackageInstanceKey,
    module: &[Intern<String>],
    grants: &[Vec<Intern<String>>],
) -> bool {
    &origin.package == package && (origin.module == module || grants.contains(&origin.module))
}

fn pointee_or_self(ty: &Ty, registry: &crate::type_registry::TypeRegistry) -> Ty {
    reference_pointee_for_type(ty, Some(registry)).unwrap_or_else(|| ty.clone())
}

impl TypedFileAst {
    pub(crate) fn can_access_method(&self, method: crate::typed::DefId) -> bool {
        self.type_registry
            .private_method(method)
            .is_none_or(
                |visibility| match (&self.semantic_origin, &visibility.origin) {
                    (Some(origin), Some(definition)) => origin_has_grant(
                        origin,
                        &definition.package,
                        &definition.module,
                        &visibility.grants,
                    ),
                    (None, None) => visibility.file == self.file_id.0,
                    _ => false,
                },
            )
    }

    pub(crate) fn can_access_member(&self, ty: &Ty, name: Intern<String>) -> bool {
        let Some(declaration) = self.type_registry.declaration_for_type(ty) else {
            return true;
        };
        let Some(grants) = declaration.private_members.get(&name) else {
            return true;
        };
        self.semantic_origin.as_ref().map_or_else(
            || {
                ty.named_instance_stripping_reference_wrappers()
                    .is_some_and(|instance| instance.declaration.file == self.file_id.0)
            },
            |origin| {
                origin_has_grant(
                    origin,
                    &declaration.key.package,
                    &declaration.key.module,
                    grants,
                )
            },
        )
    }

    pub(in crate::transform) fn can_access_representation(&self, ty: &Ty) -> bool {
        self.type_registry
            .declaration_for_type(ty)
            .is_none_or(|declaration| {
                declaration
                    .private_fields
                    .iter()
                    .all(|name| self.can_access_member(ty, *name))
            })
    }
    pub(crate) fn can_access_field(&self, base: ExprId, index: usize) -> bool {
        let Some(ty) = self.exprs.ty_of(base) else {
            return true;
        };
        let ty = pointee_or_self(ty, &self.type_registry);
        let Ty::Record { fields, .. } = self.type_registry.resolved_definition_for_type(&ty) else {
            return true;
        };
        fields
            .get(index)
            .is_none_or(|(name, _)| self.can_access_member(&ty, *name))
    }
}

pub(super) fn validate(typed: &mut TypedFileAst) {
    let sequence_lengths: HashSet<_> = typed
        .exprs
        .kind
        .iter()
        .filter_map(|kind| match kind {
            TypedExprKind::Loop(loop_) => match &loop_.kind {
                TypedLoopKind::ForIn {
                    traversal: SequenceTraversal::RuntimeView { length, .. },
                    ..
                } => Some(*length),
                _ => None,
            },
            _ => None,
        })
        .collect();
    for index in 0..typed.exprs.kind.len() {
        let id = ExprId(index as u32);
        let kind = &typed.exprs.kind[index];
        let ty = &typed.exprs.ty[index];
        let inaccessible = match kind {
            TypedExprKind::TagCall { .. } | TypedExprKind::RecordValue(_) => {
                !typed.can_access_representation(ty)
            }
            TypedExprKind::Cast { expr, .. } if typed.exprs.ty_of(*expr) != Some(ty) => {
                !typed.can_access_representation(ty)
            }
            TypedExprKind::AggregateGet { base, index } if !sequence_lengths.contains(&id) => {
                !typed.can_access_field(*base, *index)
            }
            TypedExprKind::AggregateSet { base, index, .. } => {
                !typed.can_access_field(*base, *index)
            }
            TypedExprKind::RecordSet { base, field, .. } => {
                typed.exprs.ty_of(*base).is_some_and(|ty| {
                    let ty = pointee_or_self(ty, &typed.type_registry);
                    !typed.can_access_member(&ty, *field)
                })
            }
            TypedExprKind::FnCall { target, .. } => !typed.can_access_method(*target),
            _ => false,
        };
        if inaccessible {
            // type-private-member
            let diagnostic = Diagnostic::new(
                MemberCode::Private,
                "cannot access a private member outside its defining module",
            )
            .at_span_id(typed.exprs.span[index], &typed.span_table);
            typed.exprs.facts.semantic_facts.flaws[index].push(diagnostic);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_registry::PrivateMethod;
    use crate::{DefId, FileId};
    use ast::prelude::FileSemanticOrigin;
    use ast::ty::PackageInstanceKey;

    fn origin(package: &str, module: &str) -> FileSemanticOrigin {
        FileSemanticOrigin {
            package: PackageInstanceKey::workspace(package, "1"),
            module: vec![Intern::from_ref(module)],
        }
    }

    #[test]
    fn private_method_grants_do_not_cross_package_boundaries() {
        let mut typed = TypedFileAst::new(FileId(0), ast::SpanTable::new());
        let method = DefId::named(Intern::from_ref("hidden"));
        typed.type_registry.insert_private_method(
            method,
            PrivateMethod {
                origin: Some(origin("model", "definition")),
                file: 0,
                grants: vec![vec![Intern::from_ref("client")]],
            },
        );
        for (package, module, allowed) in [
            ("model", "definition", true),
            ("model", "client", true),
            ("model", "stranger", false),
            ("other", "definition", false),
            ("other", "client", false),
        ] {
            typed.semantic_origin = Some(crate::typed::Shared::new(origin(package, module)));
            assert_eq!(
                typed.can_access_method(method),
                allowed,
                "{package}.{module}"
            );
        }
        typed.semantic_origin = None;
        assert!(!typed.can_access_method(method));
    }

    #[test]
    fn private_methods_without_origins_require_the_defining_file() {
        let mut typed = TypedFileAst::new(FileId(0), ast::SpanTable::new());
        let method = DefId::named(Intern::from_ref("hidden"));
        typed.type_registry.insert_private_method(
            method,
            PrivateMethod {
                origin: None,
                file: 0,
                grants: vec![],
            },
        );
        assert!(typed.can_access_method(method));
        typed.file_id = FileId(1);
        assert!(!typed.can_access_method(method));
        typed.file_id = FileId(0);
        typed.semantic_origin = Some(crate::typed::Shared::new(origin("model", "definition")));
        assert!(!typed.can_access_method(method));
    }
}
