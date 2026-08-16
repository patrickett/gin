use crate::prelude::*;
use typecheck::ty::Ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn lower_equality(
        &self,
        block: &BlockRef<'c, 'c>,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        ty: &Ty,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();
        let resolved = self.program.type_registry.resolved_definition_for_type(ty);
        match &resolved {
            Ty::AnonymousInteger { .. } | Ty::ResultFamily { .. } => {
                Some(block.append_op(self.mlir.build_cmpi(Predicates::EQ, lhs, rhs, loc)))
            }
            Ty::Float { .. } => Some(block.append_op(self.mlir.build_cmpf(1, lhs, rhs, loc))),
            Ty::Unit => Some(block.const_int(self.mlir, self.mlir.i1(), 1, loc)),
            Ty::Record { fields, .. } => self.lower_aggregate_equality(
                block,
                lhs,
                rhs,
                fields.iter().map(|(_, field)| field.as_ref()),
            ),
            Ty::Tuple(fields) => self.lower_aggregate_equality(block, lhs, rhs, fields.iter()),
            Ty::Union { variants, .. }
                if variants.iter().all(|variant| variant.fields.is_empty()) =>
            {
                Some(block.append_op(self.mlir.build_cmpi(Predicates::EQ, lhs, rhs, loc)))
            }
            Ty::Literal(ast::ConstValue::Int(_) | ast::ConstValue::Tag { .. }) => {
                Some(block.append_op(self.mlir.build_cmpi(Predicates::EQ, lhs, rhs, loc)))
            }
            Ty::Literal(ast::ConstValue::Float(_)) => {
                Some(block.append_op(self.mlir.build_cmpf(1, lhs, rhs, loc)))
            }
            Ty::Named { .. } => unreachable!(),
            Ty::UnresolvedLiteral(_)
            | Ty::Opaque(_)
            | Ty::Array { .. }
            | Ty::Ptr { .. }
            | Ty::Address { .. }
            | Ty::Ref { .. }
            | Ty::Union { .. }
            | Ty::Literal(_) => {
                self.emit_internal(format!(
                    "unsupported equality for `{}` reached code generation",
                    ty.format_for_hover()
                ));
                None
            }
        }
    }

    fn lower_aggregate_equality<'t>(
        &self,
        block: &BlockRef<'c, 'c>,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        fields: impl Iterator<Item = &'t Ty>,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.location();
        let mut result = block.const_int(self.mlir, self.mlir.i1(), 1, loc);
        for (index, field_ty) in fields.enumerate() {
            let mlir_ty = self.ty_to_mlir(field_ty);
            let lhs_field =
                block.append_op(self.mlir.llvm_extractvalue(lhs, index as i64, mlir_ty, loc));
            let rhs_field =
                block.append_op(self.mlir.llvm_extractvalue(rhs, index as i64, mlir_ty, loc));
            let field_equal = self.lower_equality(block, lhs_field, rhs_field, field_ty)?;
            result = block.append_op(self.mlir.build_binop(
                "arith.andi",
                result,
                field_equal,
                self.mlir.i1(),
                loc,
            ));
        }
        Some(result)
    }
}
