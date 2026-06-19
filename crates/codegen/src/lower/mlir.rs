//! MLIR utility helpers for string globals and constant value lowering.
//!
//! This module provides methods on [`CodegenContext`] for:
//!
//! - `addressof_string_global` — produces an `llvm.mlir.addressof` operation to get a pointer
//!   to a string global
//! - `lower_const_value` — lowers a compile-time [`ConstValue`] to an MLIR value

use ast::{ConstValue, HashFloat};

use crate::prelude::*;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// This returns a pointer to the global that can be used in function calls.
    pub fn addressof_string_global(
        &self,
        block: &BlockRef<'c, 'c>,
        global_name: &str,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        let global_name_id = Identifier::new(self.mlir, "global_name");
        let symbol_ref = self.mlir.symbol_ref_attr(global_name);

        let addressof_op = OperationBuilder::new("llvm.mlir.addressof", loc)
            .add_attributes(&[(global_name_id, symbol_ref)])
            .add_results(&[self.mlir.llvm_ptr()])
            .build()
            .ok()?;

        // Append the operation to the block and return the result
        Some(
            block
                .append_operation(addressof_op)
                .result(0)
                .expect("result 0 should exist on append_operation")
                .into(),
        )
    }

    /// Lower a compile-time [`ConstValue`] to an MLIR value.
    pub(crate) fn lower_const_value(
        &self,
        block: &BlockRef<'c, 'c>,
        cv: &ConstValue,
    ) -> Option<Value<'c, 'c>> {
        match cv {
            ConstValue::Int(n) => {
                let mlir_ty = if *n > i64::MAX as i128 || *n < i64::MIN as i128 {
                    self.mlir.i128()
                } else {
                    self.mlir.i64()
                };
                Some(block.const_int(self.mlir, mlir_ty, *n, self.location()))
            }
            ConstValue::Float(HashFloat(f)) => Some(block.append_op(self.mlir.const_op(
                self.mlir.f64_attr(*f),
                self.mlir.f64(),
                self.location(),
            ))),
            ConstValue::String(s) => Some(block.const_string_with_ctx(self, s, self.location())),
            _ => None,
        }
    }
}
