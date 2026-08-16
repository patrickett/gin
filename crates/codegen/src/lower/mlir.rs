//! MLIR utility helpers for string globals and constant value lowering.
//!
//! This module provides methods on [`CodegenContext`] for:
//!
//! - `addressof_string_global` — produces an `llvm.mlir.addressof` operation to get a pointer
//!   to a string global
//! - `lower_const_value` — lowers a compile-time [`ConstValue`] to an MLIR value

use ast::{ConstValue, HashFloat};

use crate::prelude::*;
use melior::ir::Region;
use melior::ir::operation::OperationBuilder;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn build_string_value(
        &self,
        block: &BlockRef<'c, 'c>,
        ptr: Value<'c, 'c>,
        len: Value<'c, 'c>,
        string_ty: &ast::Ty,
        loc: Location<'c>,
    ) -> Value<'c, 'c> {
        let undef = block.append_op(self.mlir.llvm_undef(self.ty_to_mlir(string_ty), loc));
        let with_ptr = block.append_op(self.mlir.llvm_insertvalue(undef, ptr, 0, loc));
        block.append_op(self.mlir.llvm_insertvalue(with_ptr, len, 1, loc))
    }

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

    /// Emit all registered string constants as `llvm.mlir.global` operations.
    /// Called at the end of module construction to ensure all strings referenced
    /// by `addressof_string_global` have corresponding global definitions.
    pub fn emit_string_globals(&self, module: &melior::ir::Module<'c>) {
        for (symbol_name, value) in self.strings.iter_symbols() {
            let loc = self.location();

            // Null-terminated string bytes
            let with_nul = format!("{}\0", value);
            let Ok(byte_len) = u32::try_from(with_nul.len()) else {
                continue;
            };

            let i8_type = Type::from(melior::ir::r#type::IntegerType::new(self.mlir, 8));
            let array_type = melior::dialect::llvm::r#type::array(i8_type, byte_len);

            let linkage_attr = melior::dialect::llvm::attributes::linkage(
                self.mlir,
                melior::dialect::llvm::attributes::Linkage::Internal,
            );

            let global = match OperationBuilder::new("llvm.mlir.global", loc)
                .add_attributes(&[
                    (
                        Identifier::new(self.mlir, "sym_name"),
                        self.mlir.str_attr(&symbol_name),
                    ),
                    (
                        Identifier::new(self.mlir, "global_type"),
                        melior::ir::attribute::TypeAttribute::new(array_type).into(),
                    ),
                    (
                        Identifier::new(self.mlir, "value"),
                        melior::ir::attribute::StringAttribute::new(self.mlir, &with_nul).into(),
                    ),
                    (Identifier::new(self.mlir, "linkage"), linkage_attr),
                    (
                        Identifier::new(self.mlir, "constant"),
                        melior::ir::Attribute::unit(self.mlir),
                    ),
                    (
                        Identifier::new(self.mlir, "addr_space"),
                        melior::ir::attribute::IntegerAttribute::new(
                            melior::ir::r#type::IntegerType::new(self.mlir, 32).into(),
                            0,
                        )
                        .into(),
                    ),
                ])
                .add_regions([Region::new()])
                .build()
            {
                Ok(op) => op,
                Err(_) => continue,
            };
            let _ = module.body().append_operation(global);
        }
    }

    /// Lower a compile-time [`ConstValue`] to an MLIR value.
    pub(crate) fn lower_const_value(
        &self,
        block: &BlockRef<'c, 'c>,
        cv: &ConstValue,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        match cv {
            ConstValue::Int(n) => {
                let n = ast::integer::to_i128(*n)?;
                let mlir_ty = if n > i64::MAX as i128 || n < i64::MIN as i128 {
                    self.mlir.i128()
                } else {
                    self.mlir.i64()
                };
                Some(block.const_int(self.mlir, mlir_ty, n, self.location()))
            }
            ConstValue::Float(HashFloat(f)) => Some(block.append_op(self.mlir.const_op(
                self.mlir.f64_attr(*f),
                self.mlir.f64(),
                self.location(),
            ))),
            ConstValue::String(s) => {
                Some(block.const_string_with_ctx(self, s, ty, self.location()))
            }
            _ => None,
        }
    }
}
