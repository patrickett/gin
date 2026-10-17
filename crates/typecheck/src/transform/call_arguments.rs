use ast::Ty;
use diagnostic::Diagnostic;
use diagnostic::code::OwnershipCode;

use crate::typed::{ExprId, TypedExprKind, TypedFileAst};

use super::lower_exprs::append_derived_expr;

impl TypedFileAst {
    pub(super) fn own_storage_argument(&mut self, argument: ExprId, expected: &Ty) -> ExprId {
        let Ty::OwnedStorage {
            inner: expected_element,
        } = expected
        else {
            return argument;
        };
        let actual = self.exprs.ty[argument.as_usize()].clone();
        let definition = self.type_registry.resolved_definition_for_type(&actual);
        if !matches!(
            definition,
            Ty::OwnedStorage { .. } | Ty::Ptr { .. } | Ty::Address { .. }
        ) {
            // type-owned-storage-origin
            self.exprs.facts.semantic_facts.flaws[argument.as_usize()].push(Diagnostic::new(
                OwnershipCode::StorageOrigin, "an own storage field requires transferred storage or an explicitly raw address; a borrow or direct value cannot establish ownership",
            ));
            return argument;
        }
        let expected_argument = [ast::ty::TyArg::Type(expected_element.clone())];
        let actual_argument = [ast::ty::TyArg::Type(Box::new(
            definition.pointee_ty().unwrap().clone(),
        ))];
        if crate::analysis::unify_type_args_with_registry(
            &expected_argument,
            &actual_argument,
            Some(&self.type_registry),
        )
        .is_err()
        {
            // type-owned-storage-element-mismatch
            self.exprs.facts.semantic_facts.flaws[argument.as_usize()].push(Diagnostic::new(
                OwnershipCode::StorageElementMismatch, "the storage address must have the field's element type; use an explicit raw cast to reinterpret storage",
            ));
            return argument;
        }
        if matches!(actual, Ty::OwnedStorage { .. }) {
            return argument;
        }
        let storage = expected.clone();
        append_derived_expr(
            self,
            argument,
            TypedExprKind::Cast {
                expr: argument,
                ty: storage.clone(),
            },
            storage,
        )
    }

    pub(super) fn borrow_call_argument(&mut self, mut argument: ExprId, expected: &Ty) -> ExprId {
        let Ty::Ref {
            inner: expected_inner,
            mutable,
        } = expected
        else {
            return argument;
        };
        let actual = self.exprs.ty[argument.as_usize()].clone();
        let borrows_reference_value = actual.is_ref() && actual == **expected_inner;
        if *mutable && !borrows_reference_value && self.call_argument_is_shared(argument) {
            // type-mutable-borrow-from-shared
            self.exprs.facts.semantic_facts.flaws[argument.as_usize()].push(Diagnostic::new(
                OwnershipCode::MutableBorrowFromShared,
                "a shared reference cannot supply a mutable parameter",
            ));
            return argument;
        }
        if *mutable
            && (!actual.is_ref() || borrows_reference_value)
            && self
                .exprs
                .place_of(argument)
                .flatten()
                .is_some_and(|place| !self.places[place.0 as usize].mutable)
        {
            // type-mutable-borrow-from-immutable
            self.exprs.facts.semantic_facts.flaws[argument.as_usize()].push(Diagnostic::new(
                OwnershipCode::MutableBorrowFromImmutable,
                "an immutable place cannot supply a mutable parameter",
            ));
        }
        if let Ty::Ref {
            inner,
            mutable: actual_mutable,
        } = &actual
            && actual != **expected_inner
        {
            if *mutable == *actual_mutable
                && matches!(self.exprs.kind[argument.as_usize()], TypedExprKind::Ref(_))
            {
                return argument;
            }
            argument = append_derived_expr(
                self,
                argument,
                TypedExprKind::Deref(argument),
                *inner.clone(),
            );
        }
        let ty = Ty::Ref {
            inner: Box::new(self.exprs.ty[argument.as_usize()].clone()),
            mutable: *mutable,
        };
        append_derived_expr(self, argument, TypedExprKind::Ref(argument), ty)
    }

    pub(super) fn shared_owned_storage_access(&self, value: ExprId) -> bool {
        match self.exprs.kind_of(value) {
            Some(
                TypedExprKind::SliceGet { buf: base, .. }
                | TypedExprKind::BufGet { buf: base, .. }
                | TypedExprKind::Deref(base),
            ) => {
                if self.exprs.ty_of(*base).is_some_and(|ty| {
                    matches!(
                        self.type_registry
                            .resolved_definition_for_type(ty.without_reference_wrappers()),
                        Ty::OwnedStorage { .. }
                    )
                }) {
                    self.call_argument_is_shared(*base)
                } else {
                    self.shared_owned_storage_access(*base)
                }
            }
            Some(TypedExprKind::AggregateGet { base, .. }) => {
                self.shared_owned_storage_access(*base)
            }
            _ => false,
        }
    }

    pub(super) fn call_argument_is_shared(&self, argument: ExprId) -> bool {
        if self.exprs.ty_of(argument).is_some_and(|ty| {
            crate::ty::reference_semantics_for_type(ty, Some(&self.type_registry)).is_some_and(
                |reference| {
                    reference.permission == crate::type_registry::ReferencePermission::Observe
                },
            )
        }) {
            return true;
        }
        match self.exprs.kind_of(argument) {
            Some(
                TypedExprKind::AggregateGet { base, .. }
                | TypedExprKind::Deref(base)
                | TypedExprKind::UnionPayload { expr: base, .. }
                | TypedExprKind::Cast { expr: base, .. },
            ) => self.call_argument_is_shared(*base),
            Some(TypedExprKind::SliceGet { buf, .. } | TypedExprKind::BufGet { buf, .. }) => {
                self.call_argument_is_shared(*buf)
            }
            _ => false,
        }
    }
}
