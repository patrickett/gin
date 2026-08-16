use super::{LayoutError, LayoutKind, LayoutTarget, TargetLayout};
use crate::representation::Repr;
use flask::{CompileTarget, TargetTriple};

#[test]
fn targetless_library_has_no_implicit_layout() {
    assert_eq!(
        TargetLayout::from_compile_target(&CompileTarget::Library),
        Err(LayoutError::UnsupportedTarget)
    );
}

#[test]
fn concrete_target_selects_its_layout() {
    let target = CompileTarget::Concrete(
        TargetTriple::parse("wasm32-unknown-unknown").expect("valid target triple"),
    );
    let layout = TargetLayout::from_compile_target(&target).expect("concrete target layout");
    assert_eq!(layout.target(), LayoutTarget::Wasm32);
    assert_eq!(layout.address_size(), 4);
}

#[test]
fn pointer_representation_and_index_width_are_distinct() {
    let layout = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 8, 32)
        .expect("64-bit pointer with 32-bit index");
    assert_eq!(layout.pointer_bits(), 64);
    assert_eq!(layout.index_bits(), 32);
}

#[test]
fn zero_natural_alignment_is_rejected() {
    let error = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 0, 64)
        .expect_err("zero alignment");
    assert_eq!(
        error.code(),
        "target-layout-natural-alignment-unrepresentable"
    );
    assert!(matches!(
        error,
        LayoutError::NaturalAlignmentUnrepresentable { alignment: 0, .. }
    ));
}

#[test]
fn non_power_of_two_natural_alignment_is_rejected() {
    let error = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 3, 64)
        .expect_err("non-power-of-two alignment");
    assert!(matches!(
        error,
        LayoutError::NaturalAlignmentUnrepresentable { alignment: 3, .. }
    ));
}

#[test]
fn natural_alignment_above_index_limit_is_rejected() {
    let error = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 8, 4)
        .expect_err("alignment above index-width limit");
    assert!(matches!(
        error,
        LayoutError::NaturalAlignmentUnrepresentable {
            alignment: 8,
            index_bits: 4,
            max_alignment: 4,
            ..
        }
    ));
}

#[test]
fn size_query_uses_authoritative_target_layout() {
    assert_eq!(
        TargetLayout::x86_64().query(ast::TargetQueryKind::Size, &ast::Ty::Unit, None),
        Ok(0)
    );
}

#[test]
fn alignment_query_uses_authoritative_target_layout() {
    let byte = ast::Ty::bounded_int(0.into(), 255.into());
    assert_eq!(
        TargetLayout::x86_64().query(ast::TargetQueryKind::Alignment, &byte, None),
        Ok(1)
    );
}

#[test]
fn index_bits_query_uses_index_width_not_pointer_width() {
    let layout = TargetLayout::with_address_layout(LayoutTarget::X86_64, 8, 8, 32)
        .expect("64-bit pointer with 32-bit index");
    assert_eq!(
        layout.query(ast::TargetQueryKind::IndexBits, &ast::Ty::Unit, None),
        Ok(32)
    );
}

#[test]
fn array_stride_remains_the_element_natural_stride() {
    let layout = TargetLayout::x86_64()
        .layout_repr(&Repr::Array {
            element: Box::new(Repr::Bits { width: 64 }),
            length: 3,
        })
        .expect("array layout");

    assert_eq!(layout.alignment, 8);
    assert_eq!(layout.size, 24);
    assert!(matches!(layout.kind, LayoutKind::Array { stride: 8, .. }));
}
