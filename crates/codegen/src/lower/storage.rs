use crate::prelude::*;
use typecheck::representation::Repr;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_buffer_element(
        &self,
        buffer: typecheck::ExprId,
        index: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<(Value<'c, 'c>, Ty)> {
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
        let array = Repr::array_parts(
            pointee.as_ref().unwrap_or(buffer_ty),
            Some(self.program.type_registry),
        );
        let element = array
            .as_ref()
            .map(|(element, _)| element.clone())
            .or_else(|| pointee.clone())?;
        if pointee.is_some() && !self.validate_pointee_stride(&element) {
            return None;
        }
        let base = if array.is_some() {
            if self.program.exprs.place_of(buffer).flatten().is_some() {
                self.lower_typed_place(buffer, block, symbols)?
            } else {
                let value = self.lower_typed_expr(buffer, block, symbols)?;
                let slot = block.alloca_typed(self.mlir, value.r#type(), self.location());
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
        let width = IntegerType::try_from(index.r#type()).ok()?.width();
        if width < 64 {
            let operation = if self
                .program
                .type_registry
                .integer_operation_interpretation_for_type(index_ty)
                == Some(ast::ty::IntegerInterpretation::Signed)
            {
                "arith.extsi"
            } else {
                "arith.extui"
            };
            let operation = OperationBuilder::new(operation, self.location())
                .add_operands(&[index])
                .add_results(&[self.mlir.i64()])
                .build()
                .ok()?;
            index = block.append_op(operation);
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
}
