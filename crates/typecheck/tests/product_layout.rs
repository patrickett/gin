use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::representation::Repr;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TagId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

#[test]
fn library_record_and_nested_tuple_use_product_representation() {
    let typed = check(
        "Byte is in 0...255\n\
         Word is in 0...18446744073709551615\n\
         Inner has item Byte\n\
         Outer has first Inner, second Word\n",
    );
    let outer = &typed.tags[&TagId(Intern::from_ref("Outer"))].resolved_ty;
    assert_eq!(
        Repr::derive(outer, Some(&typed.type_registry)),
        Ok(Repr::Product {
            fields: vec![
                Repr::Product {
                    fields: vec![Repr::Bits { width: 8 }],
                },
                Repr::Bits { width: 64 },
            ],
        })
    );
}

#[test]
fn equal_product_representations_keep_record_identity() {
    let typed = check(
        "Byte is in 0...255\n\
         Left has value Byte\n\
         Right has value Byte\n",
    );
    let left = &typed.tags[&TagId(Intern::from_ref("Left"))].resolved_ty;
    let right = &typed.tags[&TagId(Intern::from_ref("Right"))].resolved_ty;
    assert_eq!(
        Repr::derive(left, Some(&typed.type_registry)),
        Repr::derive(right, Some(&typed.type_registry))
    );
    assert_ne!(left.type_id(), right.type_id());
}

#[test]
fn product_layout_is_padded_in_declaration_order_and_pointer_stride_uses_it() {
    let typed = check(
        "Byte is in 0...255\n\
         Word is in 0...18446744073709551615\n\
         Padded has first Byte, second Word\n",
    );
    let padded = &typed.tags[&TagId(Intern::from_ref("Padded"))].resolved_ty;
    let repr = Repr::derive(padded, Some(&typed.type_registry)).unwrap();
    let layout = typecheck::layout::TargetLayout::x86_64()
        .layout_repr(&repr)
        .unwrap();
    assert_eq!(layout.size, 16);
    assert_eq!(layout.alignment, 8);
    assert_eq!(
        layout.kind,
        typecheck::layout::LayoutKind::Product {
            field_offsets: vec![0, 8]
        }
    );
    let pointer = ast::Ty::Ptr {
        inner: Box::new(padded.clone()),
    };
    assert_eq!(
        typecheck::layout::TargetLayout::x86_64()
            .pointee_stride(
                &pointer.pointee_ty().unwrap().clone(),
                Some(&typed.type_registry)
            )
            .unwrap(),
        16
    );
}

#[test]
fn empty_product_is_unit_and_target_address_width_is_authoritative() {
    assert_eq!(
        Repr::from_ty(&ast::Ty::Unit),
        Some(Repr::Product { fields: vec![] })
    );
    assert_eq!(
        typecheck::layout::TargetLayout::new(typecheck::layout::LayoutTarget::Wasm32)
            .address_size(),
        4
    );
}
