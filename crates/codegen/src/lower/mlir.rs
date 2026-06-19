//! MLIR utility helpers for string globals and constant value lowering.
//!
//! This module provides methods on [`CodegenContext`] for:
//!
//! - `create_string_global` — produces an `llvm.mlir.global` operation for a string literal
//! - `addressof_string_global` — produces an `llvm.mlir.addressof` operation to get a pointer
//!   to a string global
//! - `lower_const_value` — lowers a compile-time [`ConstValue`] to an MLIR value

use ast::{ConstValue, HashFloat};

use crate::prelude::*;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Produces: `llvm.mlir.global internal constant @name("value\00") : !llvm.array<N x i8>`
    pub fn create_string_global(&self, name: &str, value: &str) -> Option<Operation<'c>> {
        let loc = self.mlir.unknown_loc();

        // Null-terminated string bytes
        let with_nul = format!("{}\0", value);
        let byte_len: u32 = match with_nul.len().try_into() {
            Ok(len) => len,
            Err(_) => return None,
        };

        let i8_type = Type::from(IntegerType::new(self.mlir, 8));
        let array_type = r#type::array(i8_type, byte_len);

        let linkage_attr = melior::dialect::llvm::attributes::linkage(
            self.mlir,
            melior::dialect::llvm::attributes::Linkage::Internal,
        );

        let global = OperationBuilder::new("llvm.mlir.global", loc)
            .add_attributes(&[
                (
                    Identifier::new(self.mlir, "sym_name"),
                    self.mlir.str_attr(name),
                ),
                (
                    Identifier::new(self.mlir, "global_type"),
                    TypeAttribute::new(array_type).into(),
                ),
                (
                    Identifier::new(self.mlir, "value"),
                    StringAttribute::new(self.mlir, &with_nul).into(),
                ),
                (Identifier::new(self.mlir, "linkage"), linkage_attr),
                (
                    Identifier::new(self.mlir, "constant"),
                    Attribute::unit(self.mlir),
                ),
                (
                    Identifier::new(self.mlir, "addr_space"),
                    IntegerAttribute::new(IntegerType::new(self.mlir, 32).into(), 0).into(),
                ),
            ])
            // llvm.mlir.global requires exactly one region in the textual format,
            // even when the value is provided as an attribute.
            .add_regions([Region::new()])
            .build()
            .ok()?;

        Some(global)
    }

    /// This returns a pointer to the global that can be used in function calls.
    pub fn addressof_string_global(
        &self,
        block: &BlockRef<'c, 'c>,
        global_name: &str,
    ) -> Option<Value<'c, 'c>> {
        let loc = self.mlir.unknown_loc();
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
                Some(block.const_int(self.mlir, mlir_ty, *n))
            }
            ConstValue::Float(HashFloat(f)) => {
                Some(block.append_op(self.mlir.const_op(self.mlir.f64_attr(*f), self.mlir.f64())))
            }
            ConstValue::String(s) => Some(block.const_string_with_ctx(self, s)),
            _ => None,
        }
    }
}
