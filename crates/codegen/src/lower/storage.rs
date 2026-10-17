use crate::Slot;
use crate::prelude::*;
use typecheck::representation::Repr;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_constant_element_address(
        &self,
        base: Value<'c, 'c>,
        aggregate: &Ty,
        index: usize,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let aggregate = self.ty_to_mlir(aggregate);
        if let Ok(index) = i32::try_from(index) {
            return block.gep_typed(self, base, aggregate, &[], &[0, index], self.location());
        }
        let index = i64::try_from(index)
            .map_err(|_| {
                self.emit_internal("aggregate index exceeds the LLVM index representation");
            })
            .ok()?;
        let index = block.const_i64(self.mlir, index, self.location());
        block.gep_typed(
            self,
            base,
            aggregate,
            &[index],
            &[0, i32::MIN],
            self.location(),
        )
    }

    pub(super) fn lower_slot_value(
        &self,
        slot: &Slot<'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        if !slot.is_slot {
            if let Some(pointee) = typecheck::ty::reference_pointee_for_type(
                &slot.ty,
                Some(self.program.type_registry),
            ) && typecheck::ty::reference_pointee_for_type(ty, Some(self.program.type_registry))
                .is_none()
            {
                let value = block.load_typed(
                    self,
                    slot.value,
                    self.ty_to_mlir(&pointee),
                    self.location(),
                )?;
                return self.convert_integer_value(value, &pointee, self.ty_to_mlir(ty), block);
            }
            return self.convert_integer_value(slot.value, &slot.ty, self.ty_to_mlir(ty), block);
        }
        let reference_slot =
            typecheck::ty::reference_pointee_for_type(&slot.ty, Some(self.program.type_registry))
                .is_some();
        if reference_slot
            && typecheck::ty::reference_pointee_for_type(ty, Some(self.program.type_registry))
                .is_some()
        {
            return Some(slot.value);
        }
        let stored_ty = if reference_slot { ty } else { &slot.ty };
        let value = block.load_typed(
            self,
            slot.value,
            self.ty_to_mlir(stored_ty),
            self.location(),
        )?;
        self.convert_integer_value(value, stored_ty, self.ty_to_mlir(ty), block)
    }

    pub(super) fn slot_binding_value(
        &self,
        slot: &Slot<'c>,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        if slot.is_slot {
            block.load_typed(self, slot.value, self.ty_to_mlir(ty), loc)
        } else {
            Some(slot.value)
        }
    }

    pub(super) fn insert_aggregate(
        &self,
        block: &BlockRef<'c, 'c>,
        aggregate: Type<'c>,
        items: impl IntoIterator<Item = Option<Value<'c, 'c>>>,
    ) -> Option<Value<'c, 'c>> {
        let mut value = block.append_op(self.mlir.llvm_undef(aggregate, self.location()));
        for (index, item) in items.into_iter().enumerate() {
            value = block.append_op(self.mlir.llvm_insertvalue(
                value,
                item?,
                i64::try_from(index).ok()?,
                self.location(),
            ));
        }
        Some(value)
    }

    pub(super) fn lower_buffer_element(
        &self,
        access: typecheck::ExprId,
        buffer: typecheck::ExprId,
        index: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<(Value<'c, 'c>, Ty)> {
        let index_expression = index;
        let buffer_ty = self.program.expr(buffer)?.ty;
        let referenced_pointer =
            typecheck::ty::reference_pointee_for_type(buffer_ty, Some(self.program.type_registry))
                .filter(|ty| {
                    self.program
                        .type_registry
                        .resolved_definition_for_type(ty)
                        .pointee_ty()
                        .is_some()
                });
        let pointee =
            self.pointee_ty_for_reference_like(referenced_pointer.as_ref().unwrap_or(buffer_ty));
        let referenced_array =
            typecheck::ty::reference_pointee_for_type(buffer_ty, Some(self.program.type_registry));
        let array_ty = referenced_array.as_ref().unwrap_or(buffer_ty);
        let array = Repr::array_parts(array_ty, Some(self.program.type_registry))
            .map(|(element, length)| (element, ast::NormalExpr::from(i128::from(length))))
            .or_else(|| {
                match self
                    .program
                    .type_registry
                    .resolved_definition_for_type(array_ty)
                {
                    Ty::Array { elem, size } => Some((*elem, size)),
                    _ => None,
                }
            });
        let element = array
            .as_ref()
            .map(|(element, _)| element.clone())
            .or_else(|| pointee.clone())?;
        let zero_sized_base = self
            .target_layout
            .pointee_stride(&element, Some(self.program.type_registry))
            == Err(typecheck::layout::LayoutError::ZeroSizedPointee);
        if pointee.is_some() && !zero_sized_base && !self.validate_pointee_stride(&element) {
            return None;
        }
        let base = if array.is_some() {
            if typecheck::ty::reference_pointee_for_type(
                buffer_ty,
                Some(self.program.type_registry),
            )
            .is_some()
            {
                self.lower_typed_expr(buffer, block, symbols)?
            } else if self.program.exprs.place_of(buffer).flatten().is_some() {
                self.lower_typed_place(buffer, block, symbols)?
            } else {
                let value = self.lower_typed_expr(buffer, block, symbols)?;
                let slot = self.allocate_stack_slot(value.r#type())?;
                block.store_typed(self, slot, value, self.location())?;
                slot
            }
        } else {
            let value = self.lower_typed_expr(buffer, block, symbols)?;
            if let Some(pointer_ty) = referenced_pointer {
                block.load_typed(self, value, self.ty_to_mlir(&pointer_ty), self.location())?
            } else {
                value
            }
        };
        let index_ty = self.program.expr(index)?.ty;
        let mut index = self.lower_typed_expr(index, block, symbols)?;
        let length = match array.as_ref() {
            Some((_, length)) => Some(self.lower_array_length(length, block, symbols)?),
            None => None,
        };
        let width = IntegerType::try_from(index.r#type())
            .ok()?
            .width()
            .max(
                length
                    .as_ref()
                    .and_then(|(length, _)| IntegerType::try_from(length.r#type()).ok())
                    .map_or(64, |ty| ty.width()),
            )
            .max(64);
        let wide_ty: Type = IntegerType::new(self.mlir, width).into();
        index = self.convert_integer_value(index, index_ty, wide_ty, block)?;
        if let Some((length, length_ty)) = length {
            let length = self.convert_integer_value(length, &length_ty, wide_ty, block)?;
            if !self
                .program
                .sequence_index_is_proven(access, buffer, index_expression)
            {
                self.check_sequence_bound(block, index, length, arith_dialect::CmpiPredicate::Uge)?;
            }
        } else if let Some(length) = self
            .program
            .type_registry
            .scalar_reference_extent(buffer_ty)
        {
            self.check_array_index(block, index, length)?;
        }
        if width > 64 {
            index = block.append_op(
                OperationBuilder::new("arith.trunci", self.location())
                    .add_operands(&[index])
                    .add_results(&[self.mlir.i64()])
                    .build()
                    .ok()?,
            );
        }
        if zero_sized_base {
            return Some((base, element));
        }
        let address = block.gep_typed(
            self,
            base,
            self.ty_to_mlir(&element),
            &[index],
            &[i32::MIN],
            self.location(),
        )?;
        Some((address, element))
    }

    pub(super) fn extract_sequence_field(
        &self,
        value: Value<'c, 'c>,
        ty: &Ty,
        index: usize,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let Ty::Record { fields, .. } = self.program.type_registry.resolved_definition_for_type(ty)
        else {
            return None;
        };
        Some(block.append_op(self.mlir.llvm_extractvalue(
            value,
            index as i64,
            self.ty_to_mlir(&fields.get(index)?.1),
            self.location(),
        )))
    }

    fn check_array_index(
        &self,
        block: &BlockRef<'c, 'c>,
        index: Value<'c, 'c>,
        length: u64,
    ) -> Option<()> {
        let length = block.const_int(
            self.mlir,
            index.r#type(),
            i128::from(length),
            self.location(),
        );
        self.check_sequence_bound(block, index, length, arith_dialect::CmpiPredicate::Uge)
    }

    pub(super) fn check_sequence_bound(
        &self,
        block: &BlockRef<'c, 'c>,
        index: Value<'c, 'c>,
        length: Value<'c, 'c>,
        predicate: arith_dialect::CmpiPredicate,
    ) -> Option<()> {
        let loc = self.location();
        let comparison = block.append_op(arith_dialect::cmpi(
            self.mlir, predicate, index, length, loc,
        ));
        let failing = Region::new();
        failing.append_block(Block::new(&[]));
        let body = failing.first_block()?;
        body.append_operation(OperationBuilder::new("llvm.intr.trap", loc).build().ok()?);
        body.append_operation(scf_dialect::r#yield(&[], loc));
        block.append_scf_if(comparison, &[], failing, Region::new(), loc);
        Some(())
    }
}
