use crate::prelude::*;
use ast::integer::ContainmentProof;
use ast::ty::IntegerInterpretation;
use typecheck::{ExprId, intrinsic::IntrinsicOp};

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_integer_shift(
        &self,
        operation: IntrinsicOp,
        operands: [ExprId; 2],
        values: [Value<'c, 'c>; 2],
        result: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let sources = operands.map(|operand| self.program.expr(operand).map(|expr| expr.ty));
        let [Some(left), Some(right)] = sources else {
            return None;
        };
        let count_max = right
            .anonymous_integer_validity()?
            .domain()
            .storage_hull()
            .and_then(|hull| ast::integer::to_u128(hull.max().min(255.into())))
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or(255);
        let result_type = self.ty_to_mlir(result);
        let mut width = count_max + 1;
        for representation in [values[0].r#type(), values[1].r#type(), result_type] {
            width = width.max(IntegerType::try_from(representation).ok()?.width());
        }
        let working_type = IntegerType::new(self.mlir, width).into();
        let left = self.convert_integer_value(values[0], left, working_type, block)?;
        let right = self.convert_integer_value(values[1], right, working_type, block)?;
        let instruction =
            OperationBuilder::new(super::intrinsic_arith_name(&operation)?, self.location())
                .add_operands(&[left, right])
                .add_results(&[working_type])
                .build()
                .map_err(|error| self.emit_internal(format!("integer shift lowering: {error}")))
                .ok()?;
        self.convert_integer_value(block.append_op(instruction), result, result_type, block)
    }

    pub(super) fn lower_integer_division(
        &self,
        operation: IntrinsicOp,
        operands: [ExprId; 2],
        values: [Value<'c, 'c>; 2],
        result: &Ty,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        let sources = operands.map(|operand| self.program.expr(operand).map(|expr| expr.ty));
        let [Some(left), Some(right)] = sources else {
            return None;
        };
        let signed = matches!(
            operation,
            IntrinsicOp::BitsSignedDivide | IntrinsicOp::BitsSignedRemainder
        );
        let result_type = self.ty_to_mlir(result);
        let types = [left, right, result];
        let representations = [values[0].r#type(), values[1].r#type(), result_type];
        let mut width = 1;
        for (ty, representation) in types.into_iter().zip(representations) {
            let unsigned = self
                .program
                .type_registry
                .integer_operation_interpretation_for_type(ty)
                == Some(IntegerInterpretation::Unsigned);
            let bits = IntegerType::try_from(representation).ok()?.width();
            width = width.max(bits + u32::from(signed && unsigned));
        }
        if signed
            && self
                .program
                .type_registry
                .integer_validity_for_type(right)
                .is_none_or(|validity| {
                    validity.domain().membership((-1).into()) != ContainmentProof::Disproven
                })
        {
            width += 1;
        }
        let working_type = IntegerType::new(self.mlir, width).into();
        let left = self.convert_integer_value(values[0], left, working_type, block)?;
        let right = self.convert_integer_value(values[1], right, working_type, block)?;
        let instruction =
            OperationBuilder::new(super::intrinsic_arith_name(&operation)?, self.location())
                .add_operands(&[left, right])
                .add_results(&[working_type])
                .build()
                .map_err(|error| self.emit_internal(format!("integer division lowering: {error}")))
                .ok()?;
        self.convert_integer_value(block.append_op(instruction), result, result_type, block)
    }
}
