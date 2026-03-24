use ginc::parse::parse_from_str;
use ginc::typeck::{TyEnv, flow_analyzer::FlowAnalyzer, Ty};
use ginc::ast::Tag;
use ginc::intern::IStr;

#[test]
fn test_narrowing_after_if_early_return() {
    let src = "Maybe(x) is Some(x) or None\n\nmain:\n    val: Maybe.Some(3)\n\n    if val is Some(v)\n    return ''\nreturn\n";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);
    let result = analyzer.into_result();

    let narrowed = result.narrowed_type_string("val");
    assert_eq!(narrowed, Some("Maybe.None".to_string()),
        "After `if val is Some(v) {{ return '' }}`, val should be narrowed to Maybe.None");
}

#[test]
fn test_inrange_int32_resolves_to_32bit() {
    let src = "Int is in -2147483648...2147483647";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(IStr::new("Int".to_string())));

    assert_eq!(ty, Ty::Int(32),
        "Int is in -2147483648...2147483647 (i32 range) should resolve to Ty::Int(32), not Int(64)");
}

#[test]
fn test_inrange_int8_resolves_to_8bit() {
    let src = "SmallInt is in -128...127";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(IStr::new("SmallInt".to_string())));

    assert_eq!(ty, Ty::Int(8),
        "SmallInt is in -128...127 (i8 range) should resolve to Ty::Int(8)");
}

#[test]
fn test_inrange_int16_resolves_to_16bit() {
    let src = "MediumInt is in -32768...32767";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(IStr::new("MediumInt".to_string())));

    assert_eq!(ty, Ty::Int(16),
        "MediumInt is in -32768...32767 (i16 range) should resolve to Ty::Int(16)");
}

#[test]
fn test_range_type_also_resolves_correctly() {
    let src = "Byte is 0...255";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_tag(&Tag::Nominal(IStr::new("Byte".to_string())));

    assert_eq!(ty, Ty::Int(8),
        "Byte is 0...255 (u8 range) should resolve to Ty::Int(8)");
}
