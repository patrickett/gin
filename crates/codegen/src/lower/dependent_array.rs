use crate::prelude::*;
use ast::{ConstValue, NormalExpr};
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;
use internment::Intern;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn lower_array_length(
        &self,
        length: &NormalExpr,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<(Value<'c, 'c>, Ty)> {
        match length {
            NormalExpr::Value(ConstValue::Int(length)) => Some((
                block.const_int(
                    self.mlir,
                    self.mlir.i64(),
                    ast::integer::to_u64(*length)? as i128,
                    self.location(),
                ),
                Ty::anonymous_integer_for_width(64, false),
            )),
            NormalExpr::Var(name) => self.lower_array_length_variable(*name, block, symbols),
            _ => {
                // codegen-dependent-array-length
                self.emit_symptom(
                    DiagnosticCode::Codegen(CodegenCode::DependentArrayLength),
                    format!("cannot lower array length `{length}`"),
                );
                None
            }
        }
    }

    fn lower_array_length_variable(
        &self,
        name: Intern<String>,
        block: &BlockRef<'c, 'c>,
        symbols: &mut ScopedSymbolTable<'c>,
    ) -> Option<(Value<'c, 'c>, Ty)> {
        let mut parts = name.as_str().split(".$field");
        let root = Intern::from_ref(parts.next()?);
        let Some(slot) = symbols.get(&root) else {
            // codegen-dependent-array-length
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::DependentArrayLength),
                format!("array length `{name}` has no runtime binding"),
            );
            return None;
        };
        let mut ty = slot.ty.clone();
        let mut value = self.lower_slot_value(slot, &ty, block)?;
        for part in parts {
            let index = part.parse::<usize>().ok()?;
            if let Some(pointee) =
                typecheck::ty::reference_pointee_for_type(&ty, Some(self.program.type_registry))
            {
                value =
                    block.load_typed(self, value, self.ty_to_mlir(&pointee), self.location())?;
                ty = pointee;
            }
            let Ty::Record { fields, .. } =
                self.program.type_registry.resolved_definition_for_type(&ty)
            else {
                return None;
            };
            value = self.extract_sequence_field(value, &ty, index, block)?;
            ty = *fields.get(index)?.1.clone();
        }
        Some((value, ty))
    }
}
