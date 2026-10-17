use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::{CodegenCode, LayoutCode, RepresentationCode};
use typecheck::ExprId;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn mlir_array_length(&self, length: u64) -> Option<u32> {
        u32::try_from(length)
            .map_err(|_| {
                // array-layout-overflow
                self.emit_symptom(
                    DiagnosticCode::Layout(LayoutCode::ArrayOverflow),
                    "fixed-array length exceeds the MLIR array dimension".to_string(),
                );
            })
            .ok()
    }

    fn array_element_type(&self, ty: &Ty) -> Option<Type<'c>> {
        let Some((element, _)) =
            typecheck::representation::Repr::array_parts(ty, Some(self.program.type_registry))
        else {
            // unresolved-fixed-array-construction
            self.emit_symptom(
                DiagnosticCode::Representation(
                    RepresentationCode::FixedArrayConstructionUnresolved,
                ),
                "fixed-array construction reached codegen without array representation".into(),
            );
            return None;
        };
        Some(self.ty_to_mlir(&element))
    }

    fn lower_array_element(
        &self,
        item: ExprId,
        expected: Type<'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let element = self.lower_typed_expr(item, block, symtab)?;
        if element.r#type() != expected {
            // codegen-array-element-type-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ArrayElementTypeMismatch),
                "array element does not match its selected storage type".into(),
            );
            return None;
        }
        Some(element)
    }

    pub(super) fn lower_repeated_array(
        &self,
        init: ExprId,
        length: u64,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let length = self.mlir_array_length(length)?;
        let expected = self.array_element_type(ty)?;
        let init = self.lower_array_element(init, expected, block, symtab)?;
        let array_ty = self.ty_to_mlir(ty);
        let elements = std::iter::repeat_n(Some(init), usize::try_from(length).ok()?);
        self.insert_aggregate(block, array_ty, elements)
    }

    pub(super) fn lower_fixed_array_elements(
        &self,
        items: &[ExprId],
        length: u64,
        ty: &Ty,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        let length = self.mlir_array_length(length)?;
        if usize::try_from(length).ok() != Some(items.len()) {
            // codegen-array-length-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::ArrayLengthMismatch),
                format!(
                    "fixed array requires {length} elements, but its literal supplies {}",
                    items.len()
                ),
            );
            return None;
        }
        let expected = self.array_element_type(ty)?;
        let array_ty = self.ty_to_mlir(ty);
        let elements = items
            .iter()
            .map(|item| self.lower_array_element(*item, expected, block, symtab));
        self.insert_aggregate(block, array_ty, elements)
    }
}
