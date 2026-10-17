//! MLIR utility helpers for string globals and constant value lowering.
//!
//! This module provides methods on [`CodegenContext`] for:
//!
//! - `addressof_string_global` — produces an `llvm.mlir.addressof` operation to get a pointer
//!   to a string global
//! - `lower_const_value` — lowers a compile-time [`ConstValue`] to an MLIR value

use ast::{ConstValue, HashFloat};
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;

use crate::prelude::*;
use melior::ir::Region;
use melior::ir::operation::OperationBuilder;

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
            .map_err(|error| self.emit_internal(format!("string address lowering: {error}")))
            .ok()?;

        Some(
            block
                .append_operation(addressof_op)
                .result(0)
                .expect("result 0 should exist on append_operation")
                .into(),
        )
    }

    /// Emit all interned module constants as `llvm.mlir.global` operations.
    /// Called at the end of module construction to ensure all globals referenced
    /// during lowering have definitions.
    pub fn emit_globals(&self, module: &melior::ir::Module<'c>) {
        self.emit_string_globals(module);
        self.emit_promoted_arrays(module);
    }

    fn emit_string_globals(&self, module: &melior::ir::Module<'c>) {
        for (symbol_name, value) in self.globals.string_symbols() {
            let Some(byte_len) = self.checked_string_byte_length(value.len()) else {
                continue;
            };

            let i8_type = Type::from(melior::ir::r#type::IntegerType::new(self.mlir, 8));
            let array_type = melior::dialect::llvm::r#type::array(i8_type, byte_len);

            let value_attr = melior::ir::attribute::StringAttribute::new(self.mlir, &value).into();
            if let Err(error) = self.append_global(
                module,
                &symbol_name,
                array_type,
                Some(value_attr),
                Region::new(),
            ) {
                self.emit_internal(format!("string global `{symbol_name}` emission: {error}"));
            }
        }
    }

    pub(super) fn append_global(
        &self,
        module: &melior::ir::Module<'c>,
        name: &str,
        global_type: Type<'c>,
        value: Option<melior::ir::Attribute<'c>>,
        region: Region<'c>,
    ) -> Result<(), String> {
        let mut builder = OperationBuilder::new("llvm.mlir.global", self.location())
            .add_attributes(&[
                (
                    Identifier::new(self.mlir, "sym_name"),
                    self.mlir.str_attr(name),
                ),
                (
                    Identifier::new(self.mlir, "global_type"),
                    melior::ir::attribute::TypeAttribute::new(global_type).into(),
                ),
                (
                    Identifier::new(self.mlir, "linkage"),
                    melior::dialect::llvm::attributes::linkage(
                        self.mlir,
                        melior::dialect::llvm::attributes::Linkage::Internal,
                    ),
                ),
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
            ]);
        if let Some(value) = value {
            builder = builder.add_attributes(&[(Identifier::new(self.mlir, "value"), value)]);
        }
        let global = builder
            .add_regions([region])
            .build()
            .map_err(|error| error.to_string())?;
        module.body().append_operation(global);
        Ok(())
    }

    /// Lower a compile-time [`ConstValue`] to an MLIR value.
    pub(crate) fn lower_const_value(
        &self,
        block: &BlockRef<'c, 'c>,
        cv: &ConstValue,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        if self
            .program
            .type_registry
            .resolved_definition_for_type(ty)
            .union_literal_values()
            .is_some()
        {
            return self.lower_literal_union_value(cv, ty, block);
        }
        match cv {
            ConstValue::Proposition(holds) => Some(block.const_int(
                self.mlir,
                self.mlir.i1(),
                i128::from(*holds),
                self.location(),
            )),
            ConstValue::Int(n) => {
                let mlir_ty = self.ty_to_mlir(ty);
                if IntegerType::try_from(mlir_ty).is_err() {
                    // codegen-constant-type-mismatch
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::ConstantTypeMismatch),
                        "integer constant requires integer storage".into(),
                    );
                    return None;
                }
                let Some(bits) = super::integer_constant_bits(*n) else {
                    // codegen-integer-constant-width
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::IntegerConstantWidth),
                        "integer constant exceeds supported 128-bit storage".into(),
                    );
                    return None;
                };
                if let Some(validity) = self.program.type_registry.integer_validity_for_type(ty)
                    && (validity.domain().membership(*n)
                        == ast::integer::ContainmentProof::Disproven
                        || validity
                            .domain()
                            .storage_hull()
                            .is_some_and(|hull| !hull.contains(*n)))
                {
                    // codegen-integer-constant-domain
                    self.emit_symptom(
                        DiagnosticCode::Codegen(CodegenCode::IntegerConstantDomain),
                        "integer constant is outside its selected domain".into(),
                    );
                    return None;
                }
                Some(block.const_int(self.mlir, mlir_ty, bits, self.location()))
            }
            ConstValue::Float(HashFloat(value)) => self.lower_float_constant(block, *value, ty),
            ConstValue::Text(text) => self.lower_text_constant(block, text, ty),
            _ => None,
        }
    }

    pub(super) fn lower_float_constant(
        &self,
        block: &BlockRef<'c, 'c>,
        value: f64,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        if self.ty_to_mlir(ty) != self.mlir.f64() {
            // codegen-constant-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ConstantTypeMismatch),
                "floating constant requires floating storage".into(),
            );
            return None;
        }
        Some(block.append_op(self.mlir.const_op(
            self.mlir.f64_attr(value),
            self.mlir.f64(),
            self.location(),
        )))
    }

    pub(super) fn lower_text_constant(
        &self,
        block: &BlockRef<'c, 'c>,
        text: &str,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        let ptr = self.lower_text_address(block, text, ty)?;
        block.load_typed(self, ptr, self.ty_to_mlir(ty), self.location())
    }

    pub(super) fn checked_string_byte_length(&self, length: usize) -> Option<u32> {
        u32::try_from(length)
            .map_err(|_| {
                // codegen-string-too-large
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::StringTooLarge),
                    format!("string byte length {length} exceeds LLVM's 32-bit array limit"),
                );
            })
            .ok()
    }

    pub(super) fn lower_text_address(
        &self,
        block: &BlockRef<'c, 'c>,
        text: &str,
        ty: &ast::Ty,
    ) -> Option<Value<'c, 'c>> {
        let Ok(typecheck::representation::Repr::Array { element, length }) =
            typecheck::representation::Repr::derive(ty, Some(self.program.type_registry))
        else {
            // codegen-constant-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ConstantTypeMismatch),
                "text constant requires byte-array storage".into(),
            );
            return None;
        };
        if !matches!(*element, typecheck::representation::Repr::Bits { width: 8 })
            || usize::try_from(length).ok() != Some(text.len())
        {
            // codegen-constant-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ConstantTypeMismatch),
                "text constant does not match its selected byte-array extent".into(),
            );
            return None;
        }
        self.checked_string_byte_length(text.len())?;
        let name = self.globals.intern_string(text);
        self.addressof_string_global(block, &name, self.location())
    }
}
