use crate::prelude::*;
use ast::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_slice_address(
        &self,
        source: typecheck::ExprId,
        start: typecheck::ExprId,
        length: typecheck::ExprId,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let source_ty = self.program.expr(source)?.ty;
        let reference =
            typecheck::ty::reference_pointee_for_type(source_ty, Some(self.program.type_registry));
        let storage = self
            .program
            .type_registry
            .resolved_definition_for_type(reference.as_ref().unwrap_or(source_ty));
        let (element, extent) = match &storage {
            Ty::Array { elem, size } => ((**elem).clone(), Some(size.clone())),
            Ty::OwnedStorage { inner } => ((**inner).clone(), None),
            _ => return None,
        };
        let mut base = if reference.is_some() {
            self.lower_typed_expr(source, block, symbols)?
        } else if extent.is_some() {
            self.lower_typed_place(source, block, symbols)?
        } else {
            self.lower_typed_expr(source, block, symbols)?
        };
        if reference.is_some() && extent.is_none() {
            base = block.load_typed(self, base, self.ty_to_mlir(&storage), self.location())?;
        }
        let start_value = self.lower_slice_index(start, length, extent.as_ref(), block, symbols)?;
        if self
            .target_layout
            .pointee_stride(&element, Some(self.program.type_registry))
            == Err(typecheck::layout::LayoutError::ZeroSizedPointee)
        {
            return Some(base);
        }
        if !self.validate_pointee_stride(&element) {
            return None;
        }
        block.gep_typed(
            self,
            base,
            self.ty_to_mlir(&element),
            &[start_value],
            &[i32::MIN],
            self.location(),
        )
    }

    fn lower_slice_index(
        &self,
        start: typecheck::ExprId,
        length: typecheck::ExprId,
        extent: Option<&ast::NormalExpr>,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let start_value = self.lower_typed_expr(start, block, symbols)?;
        let length_value = self.lower_typed_expr(length, block, symbols)?;
        symbols.insert(
            internment::Intern::new(format!("$call{}", length.0)),
            length_value,
            self.program.expr(length)?.ty.clone(),
            false,
        );
        let extent = match extent {
            Some(extent) => Some(self.lower_array_length(extent, block, symbols)?),
            None => None,
        };
        let width = [
            Some(start_value),
            Some(length_value),
            extent.as_ref().map(|(value, _)| *value),
        ]
        .into_iter()
        .flatten()
        .filter_map(|value| IntegerType::try_from(value.r#type()).ok())
        .map(|ty| ty.width())
        .max()
        .unwrap_or(64)
        .max(64);
        let wide_ty = IntegerType::new(self.mlir, width).into();
        let start_value =
            self.convert_integer_value(start_value, self.program.expr(start)?.ty, wide_ty, block)?;
        let length_value = self.convert_integer_value(
            length_value,
            self.program.expr(length)?.ty,
            wide_ty,
            block,
        )?;
        if let Some((extent, ty)) = extent {
            let extent = self.convert_integer_value(extent, &ty, wide_ty, block)?;
            self.check_slice_extent(block, start_value, length_value, extent)?;
        } else {
            self.check_slice_nonnegative(block, start_value, self.program.expr(start)?.ty)?;
            self.check_slice_nonnegative(block, length_value, self.program.expr(length)?.ty)?;
        }

        let index_ty = IntegerType::new(self.mlir, self.target_layout.index_bits()).into();
        let start_value = self.convert_integer_value(
            start_value,
            &Ty::anonymous_integer_for_width(u8::try_from(width).ok()?, false),
            index_ty,
            block,
        )?;
        Some(start_value)
    }

    fn check_slice_extent(
        &self,
        block: &BlockRef<'c, 'c>,
        start: Value<'c, 'c>,
        length: Value<'c, 'c>,
        extent: Value<'c, 'c>,
    ) -> Option<()> {
        self.check_sequence_bound(block, start, extent, arith_dialect::CmpiPredicate::Ugt)?;
        let remaining = block.append_op(
            OperationBuilder::new("arith.subi", self.location())
                .add_operands(&[extent, start])
                .add_results(&[extent.r#type()])
                .build()
                .ok()?,
        );
        self.check_sequence_bound(block, length, remaining, arith_dialect::CmpiPredicate::Ugt)
    }

    fn check_slice_nonnegative(
        &self,
        block: &BlockRef<'c, 'c>,
        value: Value<'c, 'c>,
        ty: &Ty,
    ) -> Option<()> {
        if self
            .program
            .type_registry
            .integer_operation_interpretation_for_type(ty)
            == Some(ast::integer::IntegerInterpretation::Signed)
        {
            let zero = block.const_int(self.mlir, value.r#type(), 0, self.location());
            self.check_sequence_bound(block, value, zero, arith_dialect::CmpiPredicate::Slt)?;
        }
        Some(())
    }
}
