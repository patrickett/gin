use crate::prelude::*;
use ast::ty::Ty;
use diagnostic::DiagnosticCode;
use diagnostic::code::{LayoutCode, ReferenceCode, RepresentationCode, TypeCode};
use typecheck::layout::LayoutKind;
use typecheck::representation::Repr;

#[cfg(test)]
#[path = "type_cache_tests.rs"]
mod tests;

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn result_types_to_mlir(&self, ty: &Ty) -> Vec<Type<'c>> {
        match ty {
            Ty::Sequence(fields) => fields
                .iter()
                .map(|field| self.ty_to_mlir(&field.ty))
                .collect(),
            ty if self.is_empty_product(ty) => Vec::new(),
            ty => vec![self.ty_to_mlir(ty)],
        }
    }

    pub(crate) fn convert_integer_value(
        &self,
        value: Value<'c, 'c>,
        source: &Ty,
        target: Type<'c>,
        block: &BlockRef<'c, 'c>,
    ) -> Option<Value<'c, 'c>> {
        if value.r#type() == target {
            return Some(value);
        }
        let source_width = IntegerType::try_from(value.r#type()).ok()?.width();
        let target_width = IntegerType::try_from(target).ok()?.width();
        let signed = self
            .program
            .type_registry
            .nominal_integer_interpretation_for_type(source)
            .or_else(|| source.anonymous_operation_interpretation())
            .or_else(|| {
                self.program
                    .type_registry
                    .discriminant_semantics_for_type(source)
                    .map(|semantics| semantics.interpretation)
            })
            == Some(ast::ty::IntegerInterpretation::Signed);
        let operation = if source_width > target_width {
            "arith.trunci"
        } else if signed {
            "arith.extsi"
        } else {
            "arith.extui"
        };
        let operation = OperationBuilder::new(operation, self.location())
            .add_operands(&[value])
            .add_results(&[target])
            .build()
            .map_err(|error| {
                self.emit_internal(format!("integer representation conversion: {error}"))
            })
            .ok()?;
        Some(block.append_op(operation))
    }

    /// Convert a resolved `Ty` to its MLIR `Type` representation.
    pub fn ty_to_mlir(&self, ty: &Ty) -> Type<'c> {
        if let Some(mapped) = self.type_cache.borrow().get(ty).copied() {
            return mapped;
        }
        let diagnostic_count = self.symptoms.count();
        let mapped = self.uncached_ty_to_mlir(ty);
        if self.symptoms.count() == diagnostic_count {
            self.type_cache.borrow_mut().insert(ty.clone(), mapped);
        }
        mapped
    }

    fn uncached_ty_to_mlir(&self, ty: &Ty) -> Type<'c> {
        let mlir_ctx = self.mlir;
        if self.reference_pointee_ty(ty).is_some() {
            return self.repr_to_mlir(&Repr::Address { address_space: 0 });
        }
        for registry in [Some(self.program.type_registry), None] {
            if let Ok(repr) = Repr::derive(ty, registry) {
                if self.target_layout.layout_repr(&repr).is_err() {
                    self.emit_internal("target layout cannot represent this type");
                    return r#type::r#struct(mlir_ctx, &[], false);
                }
                return self.repr_to_mlir(&repr);
            }
        }
        match ty {
            Ty::Ptr { .. } | Ty::OwnedStorage { .. } | Ty::Address { .. } | Ty::Ref { .. } => {
                self.repr_to_mlir(&Repr::Address { address_space: 0 })
            }
            Ty::Proposition(_)
            | Ty::AnonymousInteger { .. }
            | Ty::ResultFamily { .. }
            | Ty::Named { .. } => {
                // type-representation-unavailable
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::Unavailable),
                    format!(
                        "type `{}` has no concrete codegen representation",
                        ty.format_for_hover()
                    ),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Float { .. } => mlir_ctx.f64(),
            Ty::Union { .. } => {
                // sum-representation-missing
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::SumMissing),
                    "tagged-sum representation is unavailable during codegen".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::UnresolvedLiteral(_) => {
                // literal-representation-unavailable
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::LiteralUnavailable),
                    "unresolved literal reached codegen before materialization".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::UnresolvedSequence { .. } => {
                // sequence-representation-unavailable
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::SequenceUnavailable),
                    "unresolved sequence reached codegen before materialization".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Literal(ast::ConstValue::Text(_)) => {
                // text-literal-requires-representation
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::TextLiteralRequired),
                    "compile-time text must be materialized as a byte array".into(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Literal(_) => {
                // literal-representation-unavailable
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::LiteralUnavailable),
                    "compile-time literal must be materialized before runtime use".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Sequence(fields) => r#type::r#struct(
                mlir_ctx,
                &fields
                    .iter()
                    .map(|field| self.ty_to_mlir(&field.ty))
                    .collect::<Vec<_>>(),
                false,
            ),
            Ty::Record { .. } | Ty::Unit => {
                // product-representation-missing
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::ProductMissing),
                    "product representation is unavailable during codegen".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Opaque(name) => {
                // unsupported-type-representation
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::Unsupported),
                    format!(
                        "opaque type `{name}` reached codegen without a physical representation"
                    ),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Array { .. } => {
                // array-representation-missing
                self.emit_symptom(
                    DiagnosticCode::Representation(RepresentationCode::ArrayMissing),
                    "fixed-array representation is unavailable during codegen".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
        }
    }

    pub(crate) fn reference_pointee_ty(&self, ty: &Ty) -> Option<Ty> {
        let definition = self.program.type_registry.resolved_definition_for_type(ty);
        if let Some(pointee) = definition.pointee_ty().filter(|pointee| pointee.is_ref()) {
            return Some(pointee.without_reference_wrappers().clone());
        }
        if let Ty::Ref { inner, .. } = ty {
            return Some(inner.without_reference_wrappers().clone());
        }
        if let Some(pointee) =
            typecheck::ty::reference_pointee_for_type(ty, Some(self.program.type_registry))
        {
            return Some(pointee);
        }

        if ty.is_ref() {
            // incompatible-reference-kind
            self.emit_symptom(
                DiagnosticCode::Type(TypeCode::Reference(ReferenceCode::KindIncompatible)),
                format!(
                    "reference type `{}` has no resolved reference semantics",
                    ty.format_for_hover()
                ),
            );
        }

        None
    }

    pub(crate) fn repr_to_mlir(&self, repr: &Repr) -> Type<'c> {
        match repr {
            Repr::Bits { width } => IntegerType::new(self.mlir, *width).into(),
            Repr::Address { address_space } => {
                melior::dialect::llvm::r#type::pointer(self.mlir, *address_space)
            }
            Repr::Product { fields } => {
                let field_types: Vec<Type<'c>> = fields
                    .iter()
                    .map(|field| self.repr_to_mlir(field))
                    .collect();
                r#type::r#struct(self.mlir, &field_types, false)
            }
            Repr::Array { element, length } => {
                let Some(length) = self.mlir_array_length(*length) else {
                    return r#type::r#struct(self.mlir, &[], false);
                };
                r#type::array(self.repr_to_mlir(element), length)
            }
            Repr::Sum { .. } => self.sum_to_mlir(repr),
        }
    }

    fn sum_to_mlir(&self, repr: &Repr) -> Type<'c> {
        let layout = match self.target_layout.layout_repr(repr) {
            Ok(layout) => layout,
            Err(error) => {
                self.emit_symptom(error.code(), error.to_string());
                return r#type::r#struct(self.mlir, &[], false);
            }
        };
        let LayoutKind::Sum {
            discriminant,
            payload_offset,
            payload_size,
            payload_alignment,
            ..
        } = layout.kind
        else {
            // sum-representation-missing
            self.emit_symptom(
                DiagnosticCode::Representation(RepresentationCode::SumMissing),
                "sum representation did not produce sum layout".to_string(),
            );
            return r#type::r#struct(self.mlir, &[], false);
        };
        if payload_size == 0 {
            return self.repr_to_mlir(&discriminant);
        }

        let discriminant_ty = self.repr_to_mlir(&discriminant);
        let mut fields = vec![discriminant_ty];
        let Some(prefix_padding) = payload_offset.checked_sub(discriminant_size(&discriminant))
        else {
            // sum-layout-overflow
            self.emit_symptom(
                DiagnosticCode::Layout(LayoutCode::SumOverflow),
                "tagged-sum payload offset precedes its discriminant".to_string(),
            );
            return r#type::r#struct(self.mlir, &[], false);
        };
        self.append_sum_bytes(&mut fields, prefix_padding);
        fields.push(self.sum_alignment_carrier(payload_alignment));
        self.append_sum_bytes(&mut fields, payload_size.saturating_sub(payload_alignment));
        let represented = payload_offset.saturating_add(payload_size);
        self.append_sum_bytes(&mut fields, layout.size.saturating_sub(represented));
        r#type::r#struct(self.mlir, &fields, false)
    }

    fn append_sum_bytes(&self, fields: &mut Vec<Type<'c>>, count: u64) {
        if count == 0 {
            return;
        }
        let Ok(count) = u32::try_from(count) else {
            // sum-layout-overflow
            self.emit_symptom(
                DiagnosticCode::Layout(LayoutCode::SumOverflow),
                "tagged-sum padding does not fit an MLIR array length".to_string(),
            );
            return;
        };
        fields.push(r#type::array(IntegerType::new(self.mlir, 8).into(), count));
    }

    fn sum_alignment_carrier(&self, alignment: u64) -> Type<'c> {
        let width = match alignment {
            0 | 1 => 8,
            2 => 16,
            3..=4 => 32,
            5..=8 => 64,
            _ => 128,
        };
        IntegerType::new(self.mlir, width).into()
    }
}

fn discriminant_size(repr: &Repr) -> u64 {
    match repr {
        Repr::Bits { width } => typecheck::layout::scalar_size_bytes(*width),
        _ => 0,
    }
}
