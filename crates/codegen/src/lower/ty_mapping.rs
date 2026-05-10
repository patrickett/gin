use crate::prelude::*;
use typeck::{Ty, ty_byte_size_static, ty_union_discriminant_size};

/// Convert a resolved `Ty` to its MLIR `Type` representation.
pub fn ty_to_mlir<'c>(ty: &Ty, ctx: &'c Context) -> Type<'c> {
    match ty {
        Ty::Int { width: 8, .. } => IntegerType::new(ctx, 8).into(),
        Ty::Int { width: 16, .. } => IntegerType::new(ctx, 16).into(),
        Ty::Int { width: 32, .. } => IntegerType::new(ctx, 32).into(),
        Ty::Int { width: 128, .. } => IntegerType::new(ctx, 128).into(),
        Ty::Int { .. } => ctx.i64(),
        Ty::Float { .. } => ctx.f64(),
        Ty::Bool => ctx.i1(),
        Ty::Union { variants, .. } => {
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty && variants.len() <= 256 {
                if variants.len() == 2 {
                    ctx.i1()
                } else {
                    IntegerType::new(ctx, 8).into()
                }
            } else if all_empty {
                let discriminant_bits = (ty_union_discriminant_size(variants.len()) * 8) as u32;
                IntegerType::new(ctx, discriminant_bits).into()
            } else {
                let discriminant_bits = (ty_union_discriminant_size(variants.len()) * 8) as u32;
                let max_fields = variants
                    .iter()
                    .map(|(_, fields)| fields.len())
                    .max()
                    .unwrap_or(0);
                let mut slot_types = vec![IntegerType::new(ctx, discriminant_bits).into()];
                for slot_idx in 0..max_fields {
                    let widest = variants
                        .iter()
                        .filter_map(|(_, fields)| fields.get(slot_idx))
                        .map(|(_, ft)| ty_byte_size_static(ft))
                        .max()
                        .unwrap_or(8);
                    let slot_ty: Type<'c> = match widest {
                        0..=1 => IntegerType::new(ctx, 8).into(),
                        2 => IntegerType::new(ctx, 16).into(),
                        3..=4 => IntegerType::new(ctx, 32).into(),
                        5..=8 => ctx.i64(),
                        _ => ctx.i128(),
                    };
                    slot_types.push(slot_ty);
                }
                r#type::r#struct(ctx, &slot_types, false)
            }
        }
        Ty::Record { .. } => {
            let fields = ty.record_fields_sorted();
            let field_types: Vec<Type<'c>> =
                fields.iter().map(|(_, ft)| ty_to_mlir(ft, ctx)).collect();
            r#type::r#struct(ctx, &field_types, false)
        }
        Ty::ConstUnion { values, .. } => {
            if values.len() <= 256 {
                if values.len() == 2 {
                    ctx.i1()
                } else {
                    IntegerType::new(ctx, 8).into()
                }
            } else if values.len() <= 65536 {
                IntegerType::new(ctx, 16).into()
            } else {
                ctx.i64()
            }
        }
        Ty::Unit | Ty::Opaque(_) => ctx.i64(),
        Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => ctx.llvm_ptr(),
        Ty::Tuple(fields) => {
            let field_types: Vec<Type<'c>> = fields.iter().map(|f| ty_to_mlir(f, ctx)).collect();
            r#type::r#struct(ctx, &field_types, false)
        }
    }
}
