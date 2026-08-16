use crate::prelude::*;
use ast::ty::Ty;
use typecheck::layout::LayoutKind;
use typecheck::representation::Repr;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Convert a resolved `Ty` to its MLIR `Type` representation.
    pub fn ty_to_mlir(&self, ty: &Ty) -> Type<'c> {
        let mlir_ctx = self.mlir;
        if self.reference_pointee_ty(ty).is_some() {
            return self.repr_to_mlir(&Repr::Address { address_space: 0 });
        }
        if let Ok(repr) = Repr::derive(ty, Some(self.program.type_registry)) {
            if self.target_layout.layout_repr(&repr).is_err() {
                self.emit_internal("target layout cannot represent this type");
                return r#type::r#struct(mlir_ctx, &[], false);
            }
            return self.repr_to_mlir(&repr);
        }
        if let Ok(repr) = Repr::derive(ty, None) {
            if self.target_layout.layout_repr(&repr).is_err() {
                self.emit_internal("target layout cannot represent this type");
                return r#type::r#struct(mlir_ctx, &[], false);
            }
            return self.repr_to_mlir(&repr);
        }
        match ty {
            Ty::Ptr { .. } | Ty::Address { .. } | Ty::Ref { .. } => {
                self.repr_to_mlir(&Repr::Address { address_space: 0 })
            }
            Ty::AnonymousInteger { .. } | Ty::ResultFamily { .. } | Ty::Named { .. } => {
                self.emit_symptom(
                    "type-representation-unavailable",
                    format!(
                        "type `{}` has no concrete codegen representation",
                        ty.format_for_hover()
                    ),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Float { .. } => mlir_ctx.f64(),
            Ty::Union { .. } => {
                self.emit_symptom(
                    "sum-representation-missing",
                    "tagged-sum representation is unavailable during codegen".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::UnresolvedLiteral(_) => mlir_ctx.i1(),
            Ty::Literal(_) => mlir_ctx.i64(),
            ty if ty.union_literal_values().is_some() => {
                let n = ty.union_literal_values().unwrap().len();
                if n <= 256 {
                    if n == 2 {
                        mlir_ctx.i1()
                    } else {
                        IntegerType::new(mlir_ctx, 8).into()
                    }
                } else if n <= 65536 {
                    IntegerType::new(mlir_ctx, 16).into()
                } else {
                    mlir_ctx.i64()
                }
            }
            Ty::Record { .. } | Ty::Tuple(_) | Ty::Unit => {
                self.emit_symptom(
                    "product-representation-missing",
                    "product representation is unavailable during codegen".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Opaque(_) => {
                self.emit_symptom(
                    "unsupported-type-representation",
                    "opaque type reached codegen without a physical representation".to_string(),
                );
                r#type::r#struct(mlir_ctx, &[], false)
            }
            Ty::Array { .. } => {
                self.emit_symptom(
                    "array-representation-missing",
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
            self.emit_symptom(
                "incompatible-reference-kind",
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
                let Ok(length) = u32::try_from(*length) else {
                    self.emit_symptom(
                        "array-layout-overflow",
                        "fixed-array length exceeds the MLIR array dimension".to_string(),
                    );
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
            self.emit_symptom(
                "sum-representation-missing",
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
            self.emit_symptom(
                "sum-layout-overflow",
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
            self.emit_symptom(
                "sum-layout-overflow",
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
        Repr::Bits { width } => u64::from(width.div_ceil(8)),
        _ => 0,
    }
}
