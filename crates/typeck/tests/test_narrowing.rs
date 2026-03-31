use ast::Tag;
use ast::parse_from_str;
use chumsky::prelude::SimpleSpan;
use internment::Intern;
use typeck::{Ty, TyEnv, flow_analyzer::FlowAnalyzer, ty_alignment, ty_byte_size_static};

#[test]
fn test_narrowing_after_if_early_return() {
    let src = "Maybe(x) is Some(x) or None\n\nmain:\n    val: Maybe.Some(3)\n\n    if val is Some(v)\n    return ''\nreturn\n";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);
    let result = analyzer.into_result();

    let narrowed = result.narrowed_type_string("val");
    assert_eq!(
        narrowed,
        Some("Maybe.None".to_string()),
        "After `if val is Some(v) {{ return '' }}`, val should be narrowed to Maybe.None"
    );
}

#[test]
fn test_inrange_int32_resolves_to_32bit() {
    let src = "Int is in -2147483648...2147483647";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("Int".to_string()),
        SimpleSpan::from(0..0),
    ));

    assert_eq!(
        ty,
        Ty::Int(32),
        "Int is in -2147483648...2147483647 (i32 range) should resolve to Ty::Int(32), not Int(64)"
    );
}

#[test]
fn test_inrange_int8_resolves_to_8bit() {
    let src = "SmallInt is in -128...127";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("SmallInt".to_string()),
        SimpleSpan::from(0..0),
    ));

    assert_eq!(
        ty,
        Ty::Int(8),
        "SmallInt is in -128...127 (i8 range) should resolve to Ty::Int(8)"
    );
}

#[test]
fn test_inrange_int16_resolves_to_16bit() {
    let src = "MediumInt is in -32768...32767";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("MediumInt".to_string()),
        SimpleSpan::from(0..0),
    ));

    assert_eq!(
        ty,
        Ty::Int(16),
        "MediumInt is in -32768...32767 (i16 range) should resolve to Ty::Int(16)"
    );
}

#[test]
fn test_range_type_also_resolves_correctly() {
    let src = "Byte is 0...255";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("Byte".to_string()),
        SimpleSpan::from(0..0),
    ));

    assert_eq!(
        ty,
        Ty::Int(8),
        "Byte is 0...255 (u8 range) should resolve to Ty::Int(8)"
    );
}

#[test]
fn test_bool_union_optimization() {
    let src = "Bool is True or False";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("Bool".to_string()),
        SimpleSpan::from(0..0),
    ));

    let size = ty_byte_size_static(&ty);
    let align = ty_alignment(&ty);

    assert_eq!(
        size, 1,
        "Bool union with 2 variants and no fields should have size = 1 (not 16)"
    );
    assert_eq!(
        align, 1,
        "Bool union with 2 variants and no fields should have alignment = 1 (not 8)"
    );
}

#[test]
fn test_three_variant_union_optimization() {
    let src = "Color is Red or Green or Blue";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("Color".to_string()),
        SimpleSpan::from(0..0),
    ));

    let size = ty_byte_size_static(&ty);
    let align = ty_alignment(&ty);

    assert_eq!(
        size, 1,
        "Color union with 3 variants and no fields should have size = 1 (not 16)"
    );
    assert_eq!(
        align, 1,
        "Color union with 3 variants and no fields should have alignment = 1 (not 8)"
    );
}

#[test]
fn test_union_with_fields_not_optimized() {
    let src = "Maybe(x) is Some(x) or None";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(
        Intern::<String>::new("Maybe".to_string()),
        SimpleSpan::from(0..0),
    ));

    let size = ty_byte_size_static(&ty);
    let align = ty_alignment(&ty);

    // Union with fields: discriminant (1 byte for 2 variants) + max field size (8 bytes) = 9 bytes
    // The discriminant is now optimized to 1 byte instead of 8 bytes!
    assert_eq!(
        size, 9,
        "Maybe union with a variant that has a field should have size = 9 (discriminant=1, field=8)"
    );
    assert_eq!(
        align, 8,
        "Maybe union with a variant that has a field should have alignment = 8 (not optimized to 1)"
    );
}
