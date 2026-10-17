use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use melior::ir::Region;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_promoted_array(
        &self,
        expression: typecheck::ExprId,
        value: ast::ConstValue,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let ty = self.program.expr(expression)?.ty;
        let name = self.globals.intern_array(ty, &value);
        self.addressof_string_global(block, &name, self.location())
    }

    pub(super) fn emit_promoted_arrays(&self, module: &melior::ir::Module<'c>) {
        self.globals.for_each_array(|name, ty, value| {
            if self.emit_promoted_array(module, &name, ty, value).is_none() {
                // codegen-array-promotion
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::ArrayPromotion),
                    "could not emit promoted array storage".into(),
                );
            }
        });
    }

    fn emit_promoted_array(
        &self,
        module: &melior::ir::Module<'c>,
        name: &str,
        ty: &ast::Ty,
        value: &ast::ConstValue,
    ) -> Option<()> {
        let region = Region::new();
        let block = region.append_block(melior::ir::Block::new(&[]));
        let value = self.lower_array_constant(&block, value, ty)?;
        block.append_operation(
            OperationBuilder::new("llvm.return", self.location())
                .add_operands(&[value])
                .build()
                .ok()?,
        );
        let global_type = self.ty_to_mlir(ty);
        self.append_global(module, name, global_type, None, region)
            .ok()
    }

    fn lower_array_constant(
        &self,
        block: &BlockRef<'c, 'c>,
        value: &ast::ConstValue,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        let mut pending = vec![(ty.clone(), value.clone(), false)];
        let mut values = Vec::new();
        for _ in 0..typecheck::array_promotion::PROMOTED_ARRAY_NODES_MAX * 2 {
            let Some((ty, value, children_ready)) = pending.pop() else {
                assert_eq!(values.len(), 1);
                return values.pop();
            };
            let ast::ConstValue::FixedArray(items) = value else {
                values.push(self.lower_const_value(block, &value, &ty)?);
                continue;
            };
            let (element, length) = typecheck::representation::Repr::array_parts(
                &ty,
                Some(self.program.type_registry),
            )?;
            if usize::try_from(length).ok()? != items.len() {
                return None;
            }
            if !children_ready {
                pending.push((ty, ast::ConstValue::FixedArray(items.clone()), true));
                pending.extend(
                    items
                        .iter()
                        .rev()
                        .map(|item| (element.clone(), item.clone(), false)),
                );
                continue;
            }
            let start = values.len().checked_sub(items.len())?;
            let array = self.insert_aggregate(
                block,
                self.ty_to_mlir(&ty),
                values[start..].iter().copied().map(Some),
            )?;
            values.truncate(start);
            values.push(array);
        }
        None
    }
}
