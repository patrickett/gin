use std::collections::HashMap;

use ast::{Expr, SpanId};
use internment::Intern;
use parser::parse_from_str;
use typeck::{
    Ty, TyEnv, flow::ConstValue, flow_analyzer::FlowAnalyzer, ty_alignment, ty_byte_size_static,
};

#[test]
fn test_when_pattern_constant_propagation() {
    let src = "Maybe[x] is Some(x) or None\n\nmain:\n    val: Maybe.Some(3)\n    out: when val is Some(v) then v else 0\n    return out\nreturn\n";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let mut analyzer = FlowAnalyzer::new(&ty_env);
    analyzer.analyze_file(&ast);
    let result = analyzer.into_result();

    let v_is_three = result.expr_contexts.values().any(|ctx| {
        matches!(
            ctx.get_constant(&Intern::<String>::from_ref("v")),
            Some(ConstValue::Int(3))
        )
    });
    assert!(
        v_is_three,
        "`when val is Some(v)` should propagate val's payload so v = 3 inside the arm"
    );
}

#[test]
fn test_narrowing_after_if_early_return() {
    let src = "Maybe[x] is Some(x) or None\n\nmain:\n    val: Maybe.Some(3)\n\n    if val is Some(v)\n    return ''\nreturn\n";

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
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Int"),
        SpanId::INVALID,
    ));

    assert_eq!(
        ty,
        Ty::Int {
            width: 32,
            signed: true,
            value: None
        },
        "Int is in -2147483648...2147483647 (i32 range) should resolve to Ty::Int {{ width: 32, signed: true }}"
    );
}

#[test]
fn test_inrange_int8_resolves_to_8bit() {
    let src = "SmallInt is in -128...127";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("SmallInt"),
        SpanId::INVALID,
    ));

    assert_eq!(
        ty,
        Ty::Int {
            width: 8,
            signed: true,
            value: None
        },
        "SmallInt is in -128...127 (i8 range) should resolve to Ty::Int {{ width: 8, signed: true }}"
    );
}

#[test]
fn test_inrange_int16_resolves_to_16bit() {
    let src = "MediumInt is in -32768...32767";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("MediumInt"),
        SpanId::INVALID,
    ));

    assert_eq!(
        ty,
        Ty::Int {
            width: 16,
            signed: true,
            value: None
        },
        "MediumInt is in -32768...32767 (i16 range) should resolve to Ty::Int {{ width: 16, signed: true }}"
    );
}

#[test]
fn test_range_type_also_resolves_correctly() {
    let src = "Byte is 0...255";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Byte"),
        SpanId::INVALID,
    ));

    assert_eq!(
        ty,
        Ty::Int {
            width: 8,
            signed: false,
            value: None
        },
        "Byte is 0...255 (u8 range) should resolve to Ty::Int {{ width: 8, signed: false }}"
    );
}

#[test]
fn test_bool_union_optimization() {
    let src = "Bool is True or False";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Bool"),
        SpanId::INVALID,
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
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Color"),
        SpanId::INVALID,
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

// ── ConstUnion (string literal union) tests ──

#[test]
fn test_string_literal_union_resolves_to_const_union() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("LogLevel"),
        SpanId::INVALID,
    ));

    assert!(
        ty.is_const_union(),
        "LogLevel should resolve to ConstUnion, got: {ty:?}"
    );

    if let Ty::ConstUnion { name, base, values } = &ty {
        assert_eq!(name.as_str(), "LogLevel");
        assert_eq!(values.len(), 4);
        assert!(matches!(values[0], ConstValue::String(ref s) if s == "debug"));
        assert!(matches!(values[1], ConstValue::String(ref s) if s == "info"));
        assert!(matches!(values[2], ConstValue::String(ref s) if s == "warn"));
        assert!(matches!(values[3], ConstValue::String(ref s) if s == "error"));
        // base should be Str (record with pointer + len)
        assert!(base.is_record());
        if let Ty::Record { name: rn, .. } = base.as_ref() {
            assert_eq!(rn.as_str(), "Str");
        }
    } else {
        panic!("Expected ConstUnion");
    }
}

#[test]
fn test_const_union_size_and_alignment() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("LogLevel"),
        SpanId::INVALID,
    ));

    let size = ty_byte_size_static(&ty);
    let align = ty_alignment(&ty);

    assert_eq!(
        size, 1,
        "ConstUnion with 4 values should have size = 1 (i8 discriminant)"
    );
    assert_eq!(
        align, 1,
        "ConstUnion should have alignment = 1 (small integer)"
    );
}

#[test]
fn test_const_union_two_values_uses_i1() {
    let src = "Flag is 'on' or 'off'";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Flag"),
        SpanId::INVALID,
    ));

    let size = ty_byte_size_static(&ty);
    assert_eq!(
        size, 1,
        "Flag with 2 values should have size = 1 (i1, still stored as 1 byte)"
    );
}

#[test]
fn test_const_union_variant_map() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);

    // Each string value should be findable via lookup_variant
    let debug = ty_env.lookup_variant(Intern::<String>::new("debug".to_string()));
    assert!(debug.is_some(), "'debug' should be a valid variant");
    if let Some((union, disc, fields)) = debug {
        assert_eq!(union.as_str(), "LogLevel");
        assert_eq!(disc, 0);
        assert!(
            fields.is_empty(),
            "ConstUnion variants have no payload fields"
        );
    }

    let error = ty_env.lookup_variant(Intern::<String>::new("error".to_string()));
    assert!(error.is_some(), "'error' should be a valid variant");
    if let Some((_union, disc, _)) = error {
        assert_eq!(disc, 3);
    }

    // Unknown value should not be found
    let unknown = ty_env.lookup_variant(Intern::<String>::new("unknown".to_string()));
    assert!(unknown.is_none(), "'unknown' should not be a valid variant");
}

#[test]
fn test_const_union_all_variants() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let union_name = Intern::<String>::from_ref("LogLevel");

    let variants = ty_env.all_variants_of(union_name);
    assert_eq!(variants.len(), 4);

    let names: Vec<String> = variants.iter().map(|v| v.to_string()).collect();
    assert!(
        names.contains(&"debug".to_string()),
        "variants should include 'debug': {names:?}"
    );
    assert!(
        names.contains(&"info".to_string()),
        "variants should include 'info': {names:?}"
    );
}

#[test]
fn test_nothing_is_unit() {
    let src = "Nothing is ()";
    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Nothing"),
        SpanId::INVALID,
    ));

    assert_eq!(ty, Ty::Unit, "Nothing is () should resolve to Ty::Unit");

    let size = ty_byte_size_static(&ty);
    let align = ty_alignment(&ty);
    assert_eq!(size, 0, "Unit should have size 0");
    assert_eq!(align, 1, "Unit should have alignment 1");
}

#[test]
fn test_const_union_unifies_with_itself() {
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";
    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("LogLevel"),
        SpanId::INVALID,
    ));

    let mut bindings = HashMap::new();
    assert!(
        typeck::check::ty_unifies_with(&ty, &ty, &mut bindings),
        "ConstUnion should unify with itself"
    );
}

#[test]
fn test_const_union_rejects_str() {
    // A bare Str should NOT unify with a ConstUnion.
    let src = "LogLevel is 'debug' or 'info' or 'warn' or 'error'";
    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let loglevel = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("LogLevel"),
        SpanId::INVALID,
    ));

    let str_ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Str"),
        SpanId::INVALID,
    ));

    let mut bindings = HashMap::new();
    assert!(
        !typeck::check::ty_unifies_with(&str_ty, &loglevel, &mut bindings),
        "Str should NOT unify with ConstUnion"
    );
    assert!(
        !typeck::check::ty_unifies_with(&loglevel, &str_ty, &mut bindings),
        "ConstUnion should NOT unify with Str"
    );
}

#[test]
fn test_const_union_rejects_wrong_subset() {
    let src = "
LogLevel is 'debug' or 'info' or 'warn' or 'error'
SubLevel is 'debug' or 'error'
";
    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let loglevel = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("LogLevel"),
        SpanId::INVALID,
    ));
    let sublevel = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("SubLevel"),
        SpanId::INVALID,
    ));

    let mut bindings = HashMap::new();
    // SubLevel (values in LogLevel) should unify with LogLevel
    assert!(
        typeck::check::ty_unifies_with(&sublevel, &loglevel, &mut bindings),
        "SubLevel ('debug', 'error') should unify with LogLevel (all values present)"
    );

    // LogLevel has values not in SubLevel, so LogLevel should NOT unify with SubLevel
    let mut bindings2 = HashMap::new();
    assert!(
        !typeck::check::ty_unifies_with(&loglevel, &sublevel, &mut bindings2),
        "LogLevel has more values than SubLevel, should NOT unify"
    );
}

#[test]
fn test_union_with_fields_not_optimized() {
    let src = "Maybe[x] is Some(x) or None";

    let ast = parse_from_str(src);
    let ty_env = TyEnv::from_file_ast(&ast);
    let ty = ty_env.resolve_type_expr(&Expr::TypeNominal(
        Intern::<String>::from_ref("Maybe"),
        SpanId::INVALID,
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
