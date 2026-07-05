use crate::prelude::*;
use ast::ConstValue;
use ast::ty::Ty;
use typecheck::compile_time_trait::trait_field_for_ty;

impl<'a, 'c> CodegenContext<'a, 'c> {
    /// Discriminant byte size for a union with the given number of variants.
    fn union_discriminant_byte_size(num_variants: usize) -> usize {
        if num_variants <= 256 {
            1
        } else if num_variants <= 65536 {
            2
        } else {
            8
        }
    }

    /// Query the `Sized.size` trait field for a type and extract the byte count.
    ///
    /// Returns `None` when the trait is not in scope, the type is dynamic-sized,
    /// or the trait query fails. Callers should fall back to a sensible default.
    fn sized_byte_size(&self, ty: &Ty) -> Option<usize> {
        let typed = self.typed_ast?;
        let registry = self.trait_registry?;

        // Ty::Literal doesn't route through the Sized blanket cleanly since its
        // Reflectable.shape wraps the value in Record { name: "Literal", ty: <cv> }
        // which compute_size can't handle. Handle inline with rough estimates
        // (16 for strings, 8 for everything else) to avoid falling back to 0.
        //
        // TODO: Handle each literal type properly:
        //   - ConstValue::String → actual Str struct size (16 bytes, but should come from Sized)
        //   - ConstValue::Int(n) → depends on bit width (could be i128)
        //   - ConstValue::Float(_) → always 8 bytes
        //   - ConstValue::Tag { args, .. } → depends on args
        //   - ConstValue::Record { fields, .. } → sum of field sizes
        //   - ConstValue::List(items) → depends on element type + length
        //   The Sized trait should handle these, but Ty::Literal's Reflectable.shape
        //   produces a Record { name: "Literal", ty: <cv> } that compute_size can't match.
        if let Ty::Literal(cv) = ty {
            return Some(match cv {
                ConstValue::String(_) => 16,
                _ => 8,
            });
        }
        let cv = trait_field_for_ty("Sized", "size", ty, typed, registry)?;
        match cv {
            ConstValue::Tag { name, args, .. } if name.as_str() == "Const" => {
                args.first().and_then(|a| match a {
                    ConstValue::Int(n) => usize::try_from(*n).ok(),
                    _ => None,
                })
            }
            _ => None,
        }
    }

    /// Convert a resolved `Ty` to its MLIR `Type` representation.
    pub fn ty_to_mlir(&self, ty: &Ty) -> Type<'c> {
        let mlir_ctx = self.mlir;
        match ty {
            Ty::Int { width: 8, .. } => IntegerType::new(mlir_ctx, 8).into(),
            Ty::Int { width: 16, .. } => IntegerType::new(mlir_ctx, 16).into(),
            Ty::Int { width: 32, .. } => IntegerType::new(mlir_ctx, 32).into(),
            Ty::Int { width: 128, .. } => IntegerType::new(mlir_ctx, 128).into(),
            Ty::Int { .. } => mlir_ctx.i64(),
            Ty::Float { .. } => mlir_ctx.f64(),
            Ty::Union { variants, .. } => {
                let all_empty = variants.iter().all(|v| v.fields.is_empty());
                let disc_bytes = Self::union_discriminant_byte_size(variants.len());
                if all_empty && variants.len() <= 256 {
                    if variants.len() == 2 {
                        mlir_ctx.i1()
                    } else {
                        IntegerType::new(mlir_ctx, 8).into()
                    }
                } else if all_empty {
                    let discriminant_bits = (disc_bytes * 8) as u32;
                    IntegerType::new(mlir_ctx, discriminant_bits).into()
                } else {
                    let discriminant_bits = (disc_bytes * 8) as u32;
                    let max_fields = variants.iter().map(|v| v.fields.len()).max().unwrap_or(0);
                    let mut slot_types = vec![IntegerType::new(mlir_ctx, discriminant_bits).into()];
                    for slot_idx in 0..max_fields {
                        let widest = variants
                            .iter()
                            .filter_map(|v| v.fields.get(slot_idx))
                            .map(|(_, ft)| self.sized_byte_size(ft).unwrap_or(8))
                            .max()
                            .unwrap_or(8);
                        let slot_ty: Type<'c> = match widest {
                            0..=1 => IntegerType::new(mlir_ctx, 8).into(),
                            2 => IntegerType::new(mlir_ctx, 16).into(),
                            3..=4 => IntegerType::new(mlir_ctx, 32).into(),
                            5..=8 => mlir_ctx.i64(),
                            _ => mlir_ctx.i128(),
                        };
                        slot_types.push(slot_ty);
                    }
                    r#type::r#struct(mlir_ctx, &slot_types, false)
                }
            }
            Ty::Record { fields, .. } => {
                // Fields in declaration order — the language's type system handles layout.
                let field_types: Vec<Type<'c>> =
                    fields.iter().map(|(_, ft)| self.ty_to_mlir(ft)).collect();
                r#type::r#struct(mlir_ctx, &field_types, false)
            }
            Ty::Literal(_) => {
                // Literal types without union values are single values (e.g. `5`).
                mlir_ctx.i64()
            }
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
            Ty::Unit => mlir_ctx.i64(),
            Ty::Opaque(name) if name.as_str() == "Str" || name.as_str() == "String" => {
                mlir_ctx.string_type()
            }
            Ty::Opaque(_) => mlir_ctx.i64(),
            Ty::Array { .. } | Ty::Ptr { .. } => mlir_ctx.llvm_ptr(),
            Ty::Ref { inner, .. } => self.ty_to_mlir(inner),
            Ty::Tuple(fields) => {
                let field_types: Vec<Type<'c>> =
                    fields.iter().map(|f| self.ty_to_mlir(f)).collect();
                r#type::r#struct(mlir_ctx, &field_types, false)
            }
        }
    }
}
