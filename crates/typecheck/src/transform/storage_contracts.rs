use diagnostic::Diagnostic;
use diagnostic::code::OwnershipCode;

use crate::ty::{Ty, reference_pointee_for_type};
use crate::typed::{TypedExprKind, TypedFileAst};

pub(super) fn validate(typed: &mut TypedFileAst) {
    for index in 0..typed.exprs.kind.len() {
        let kind = &typed.exprs.kind[index];
        let ty = &typed.exprs.ty[index];
        let mut flaws = Vec::new();
        if typed.type_registry.storage_requires_borrow_tracking(ty) {
            // type-storage-borrowed-payload
            flaws.push(Diagnostic::new(
                OwnershipCode::StorageBorrowedPayload,
                "managed storage carrying borrowed values or allocator state requires tracking the stored loans",
            ));
        }
        let establishes_storage = match kind {
            TypedExprKind::Cast { expr, .. } => typed.exprs.ty_of(*expr) != Some(ty),
            TypedExprKind::Deref(source) | TypedExprKind::BufGet { buf: source, .. } => {
                typed.exprs.ty_of(*source).is_some_and(|source| {
                    let source = reference_pointee_for_type(source, Some(&typed.type_registry))
                        .unwrap_or_else(|| source.clone());
                    matches!(
                        typed.type_registry.resolved_definition_for_type(&source),
                        Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. }
                    )
                })
            }
            _ => false,
        };
        if establishes_storage {
            let target = reference_pointee_for_type(ty, Some(&typed.type_registry))
                .unwrap_or_else(|| ty.clone());
            if typed.type_registry.owns_storage_value(&target)
                && !typed.owns_storage_origin_authority(&target)
            {
                // type-storage-authority
                flaws.push(Diagnostic::new(
                    OwnershipCode::StorageAuthority,
                    "only the storage authority can establish ownership from a raw representation",
                ));
            }
        }
        for flaw in flaws {
            let already = typed.exprs.facts.semantic_facts.flaws[index]
                .iter()
                .any(|existing| existing.code == flaw.code);
            if !already {
                let diagnostic = flaw.at_span_id(typed.exprs.span[index], &typed.span_table);
                typed.exprs.facts.semantic_facts.flaws[index].push(diagnostic);
            }
        }
    }
}
