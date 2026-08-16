use ast::ConstValue;
use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{PackageTransformOptions, TransformCtx, transform, transform_package};
use typecheck::ty::Ty;
use typecheck::{BindBody, DefId, ExprId, FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    let ast = TokenCursor::parse_source(source);
    transform(&ast, FileId(0), &TransformCtx::new())
}

fn body(typed: &TypedFileAst, name: &str) -> ExprId {
    match &typed.defs[&DefId(Intern::new(name.to_string()))].body {
        BindBody::Expr(expr) => *expr,
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()).unwrap(),
        BindBody::Extern => panic!("expected body"),
    }
}

fn flaw_codes(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, flaw)| flaw.code.slug().to_string())
        .collect()
}

#[test]
fn annotation_materializes_literal_with_nominal_identity_and_value() {
    let typed = check("LibraryInt is in 0...255\nvalue LibraryInt: 42\n");
    let expr = body(&typed, "value");
    let declared = &typed.tags[&typecheck::TagId(Intern::from_ref("LibraryInt"))].resolved_ty;

    assert_eq!(
        typed.exprs.ty[expr.as_usize()].type_id(),
        declared.type_id()
    );
    assert_eq!(
        typed.exprs.const_value[expr.as_usize()],
        Some(ConstValue::Int(42.into()))
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn nominal_integer_metadata_separates_validity_interpretation_and_representation() {
    let typed = check("LibraryInt is in 250...260\n");
    let declared = &typed.tags[&typecheck::TagId(Intern::from_ref("LibraryInt"))].resolved_ty;
    let semantics = typed
        .type_registry
        .nominal_integer_semantics_for_type(declared)
        .expect("integer declaration metadata");

    assert_eq!(
        semantics.interpretation,
        ast::integer::IntegerInterpretation::Unsigned
    );
    assert!(semantics.validity.domain().contains(i256::I256::from(250)));
    assert!(semantics.validity.domain().contains(i256::I256::from(260)));
    assert!(!semantics.validity.domain().contains(i256::I256::from(261)));
    assert_eq!(
        ast::integer::resolve_representation(&semantics.validity, &semantics.representation_rule)
            .unwrap()
            .width()
            .get(),
        9
    );
}

#[test]
fn bits_attribute_selects_exact_width_without_changing_validity() {
    let typed = check("#bits(16)\nWide is in 0...256\n");
    let declared = &typed.tags[&typecheck::TagId(Intern::from_ref("Wide"))].resolved_ty;
    let semantics = typed
        .type_registry
        .nominal_integer_semantics_for_type(declared)
        .expect("integer semantics");

    assert!(semantics.validity.domain().contains(i256::I256::from(256)));
    assert_eq!(
        typed.type_registry.integer_width_for_type(declared),
        Some(16)
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn bits_attribute_rejects_width_smaller_than_validity() {
    let typed = check("#bits(8)\nWide is in 0...256\n");

    assert!(flaw_codes(&typed).contains(&"type-integer-representation-too-small".to_string()));
}

#[test]
fn bits_attribute_rejects_zero_width() {
    let typed = check("#bits(0)\nZero is in 0...1\n");

    assert!(flaw_codes(&typed).contains(&"type-integer-representation-width-invalid".to_string()));
}

#[test]
fn bits_attribute_rejects_runtime_width_above_128() {
    let typed = check("#bits(129)\nWide is in 0...1\n");

    assert!(flaw_codes(&typed).contains(&"type-integer-runtime-width-unsupported".to_string()));
}

#[test]
fn bits_attribute_rejects_unresolved_width() {
    let typed = check("#bits(width)\nWide is in 0...1\n");

    assert!(flaw_codes(&typed).contains(&"type-integer-representation-width-invalid".to_string()));
}

#[test]
fn occurrence_integer_knowledge_is_separate_from_nominal_type_identity() {
    let typed = check(
        "Window is in 0...10\n\
         known Window: 5\n\
         passthrough(value Window) Window: value\n",
    );
    let window = &typed.tags[&typecheck::TagId(Intern::from_ref("Window"))].resolved_ty;
    let known = body(&typed, "known");
    let passthrough = body(&typed, "passthrough");

    assert_eq!(typed.exprs.ty[known.as_usize()].type_id(), window.type_id());
    assert!(matches!(
        &typed.exprs.integer_knowledge[known.as_usize()],
        Some(ast::integer::IntegerKnowledge::Exact(
            ast::integer::CanonicalIntegerExpr::Value(value)
        )) if *value == i256::I256::from(5)
    ));
    assert_eq!(
        typed.exprs.ty[passthrough.as_usize()].type_id(),
        window.type_id()
    );
    assert_eq!(
        typed.exprs.integer_knowledge[passthrough.as_usize()],
        Some(ast::integer::IntegerKnowledge::Unknown)
    );
}

#[test]
fn positive_occurrence_knowledge_does_not_change_nominal_signed_interpretation() {
    let typed = check("Signed is in -10...10\nvalue Signed: 5\n");
    let declared = &typed.tags[&typecheck::TagId(Intern::from_ref("Signed"))].resolved_ty;
    let expr = body(&typed, "value");

    assert!(matches!(
        typed.exprs.integer_knowledge[expr.as_usize()],
        Some(ast::integer::IntegerKnowledge::Exact(
            ast::integer::CanonicalIntegerExpr::Value(value)
        )) if value == 5.into()
    ));
    assert_eq!(
        typed
            .type_registry
            .nominal_integer_interpretation_for_type(declared),
        Some(ast::integer::IntegerInterpretation::Signed)
    );
    assert_eq!(
        typed.exprs.ty[expr.as_usize()].type_id(),
        declared.type_id()
    );
}

#[test]
fn compile_time_literal_above_128_bits_preserves_exact_value() {
    let literal = "1606938044258990275541962092341162602522202993782792835301376";
    let typed = check(&format!("value: {literal}\n"));
    let expr = body(&typed, "value");
    let expected = i256::I256::from_str_radix(literal, 10).expect("I256 literal");

    assert_eq!(
        typed.exprs.const_value[expr.as_usize()],
        Some(ConstValue::Int(expected))
    );
    assert!(matches!(
        typed.exprs.integer_knowledge[expr.as_usize()],
        Some(ast::integer::IntegerKnowledge::Exact(
            ast::integer::CanonicalIntegerExpr::Value(value)
        )) if value == expected
    ));
}

#[test]
fn runtime_materialization_of_wide_source_integer_is_rejected() {
    let literal = "1606938044258990275541962092341162602522202993782792835301376";
    let typed = check(&format!(
        "Wide is in 0...{literal}\nvalue Wide: {literal}\n"
    ));

    assert!(flaw_codes(&typed).contains(&"type-integer-runtime-width-unsupported".to_string()));
}

#[test]
fn compile_time_addition_uses_mathematical_result_width() {
    let typed = check("sum := 250 + 10\n");
    let expr = body(&typed, "sum");

    assert!(matches!(
        typed.exprs.integer_knowledge[expr.as_usize()],
        Some(ast::integer::IntegerKnowledge::Exact(
            ast::integer::CanonicalIntegerExpr::Value(value)
        )) if value == 260.into()
    ));
    assert_eq!(
        typed.exprs.ty[expr.as_usize()].anonymous_integer_width(),
        Some(9)
    );
}

#[test]
fn parameter_and_return_types_materialize_literals() {
    let typed = check(
        "LibraryInt is in 0...255\n\
         take(value LibraryInt) LibraryInt: value\n\
         result LibraryInt: take(42)\n\
         make() LibraryInt: 43\n",
    );
    let call = body(&typed, "result");
    let argument = match &typed.exprs.kind[call.as_usize()] {
        typecheck::TypedExprKind::FnCall {
            args: Some(args), ..
        } => args[0],
        kind => panic!("expected call, got {kind:?}"),
    };
    let expected = typed.tags[&typecheck::TagId(Intern::from_ref("LibraryInt"))]
        .resolved_ty
        .type_id();

    assert_eq!(typed.exprs.ty[argument.as_usize()].type_id(), expected);
    assert_eq!(
        typed.exprs.ty[body(&typed, "make").as_usize()].type_id(),
        expected
    );
}

#[test]
fn renamed_default_materializes_unconstrained_literal() {
    let source = "#default(IntegerLiteral)\nRenamed is in -128...127\nvalue: 42\n";
    let parsed = TokenCursor::parse_source(source);
    assert_eq!(
        parsed.tags[&Intern::from_ref("Renamed")]
            .attributes
            .default_literal,
        Some(ast::ty::LiteralKind::Integer)
    );
    let typed = transform(&parsed, FileId(0), &TransformCtx::new());
    let expr = body(&typed, "value");
    let expected = typed.tags[&typecheck::TagId(Intern::from_ref("Renamed"))]
        .resolved_ty
        .type_id();

    assert_eq!(typed.exprs.ty[expr.as_usize()].type_id(), expected);
    assert_eq!(
        typed.exprs.const_value[expr.as_usize()],
        Some(ConstValue::Int(42.into()))
    );
}

#[test]
fn compile_time_integer_preserves_i256_value_beyond_the_runtime_default() {
    let source = "#default(IntegerLiteral)\nSignedDefault is in -9223372036854775808...9223372036854775807\nwide := 9223372036854775808\n";
    let typed = transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    );
    let expr = body(&typed, "wide");

    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(
        typed.exprs.const_value[expr.as_usize()],
        Some(ConstValue::Int(i256::I256::from(1_u128 << 63)))
    );
    assert!(matches!(
        typed.exprs.ty[expr.as_usize()],
        Ty::AnonymousInteger { .. }
    ));
}

#[test]
fn missing_and_ambiguous_defaults_are_diagnosed() {
    let missing = check("value: 42\n");
    assert!(!flaw_codes(&missing).contains(&"type-missing-integer-literal-default".to_string()));

    let ambiguous = check(
        "#default(IntegerLiteral)\nFirst is in 0...255\n\
         #default(IntegerLiteral)\nSecond is in 0...255\n\
         value: 42\n",
    );
    assert!(
        !flaw_codes(&ambiguous).contains(&"type-ambiguous-integer-literal-default".to_string())
    );
}

#[test]
fn bounds_and_negative_validity_are_diagnosed() {
    let out_of_range = check("Tiny is in 0...255\nvalue Tiny: 256\n");
    assert!(flaw_codes(&out_of_range).contains(&"type-integer-literal-out-of-range".to_string()));

    let unsigned_negative = check("Tiny is in 0...255\nvalue Tiny: -1\n");
    assert!(
        flaw_codes(&unsigned_negative)
            .contains(&"type-negative-integer-literal-rejected".to_string())
    );

    let signed = check("SignedTiny is in -128...127\nvalue SignedTiny: -1\n");
    let expr = body(&signed, "value");
    assert_eq!(
        signed.exprs.const_value[expr.as_usize()],
        Some(ConstValue::Int((-1).into()))
    );
    assert!(signed.all_flaws().is_empty(), "{:?}", signed.all_flaws());

    let empty = check("Invalid is in 10...2\nvalue Invalid: 1\n");
    assert!(
        flaw_codes(&empty).contains(&"type-integer-domain-empty".to_string()),
        "expected empty-domain diagnostic for 10...2 declaration"
    );
}

#[test]
fn unbounded_source_refinement_requires_finite_runtime_bounds() {
    let typed = check("main:\n    value is > 0: 2\n    return value\n");

    assert!(
        flaw_codes(&typed).contains(&"type-integer-refinement-requires-finite-bounds".to_string()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn bounded_source_refinement_normalizes_strict_endpoints() {
    let typed = check("main:\n    value is > 0 and < 10: 2\n    return value\n");
    let index = typed
        .exprs
        .kind
        .iter()
        .position(|kind| {
            matches!(kind, typecheck::TypedExprKind::Bind { name, .. } if name.as_str() == "value")
        })
        .expect("value bind");
    let validity = typed.exprs.ty[index]
        .anonymous_integer_validity()
        .expect("anonymous integer validity");

    assert!(!validity.domain().contains(0.into()));
    assert!(validity.domain().contains(1.into()));
    assert!(validity.domain().contains(9.into()));
    assert!(!validity.domain().contains(10.into()));
    assert_eq!(typed.exprs.ty[index].anonymous_integer_width(), Some(4));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn operator_operands_use_the_nominal_peer_or_visible_default() {
    let typed = check(
        "#default(IntegerLiteral)\n\
         Word is in 0...255\n\
         #intrinsic(BitsAdd)\n\
         #operator(Add)\n\
         plus(left Word, right Word) Word extern\n\
         right(value Word) Word: value + 1\n\
         left(value Word) Word: 1 + value\n\
         both := 1 + 2\n",
    );
    let expected = typed.tags[&typecheck::TagId(Intern::from_ref("Word"))]
        .resolved_ty
        .type_id();

    for name in ["right", "left"] {
        let expr = body(&typed, name);
        let args = match &typed.exprs.kind[expr.as_usize()] {
            typecheck::TypedExprKind::FnCall {
                args: Some(args), ..
            } => args,
            kind => panic!("expected resolved operator call, got {kind:?}"),
        };
        assert!(args.iter().all(|arg| {
            typed.exprs.ty[arg.as_usize()]
                .without_reference_wrappers()
                .type_id()
                == expected
        }));
    }

    let both = body(&typed, "both");
    assert_eq!(typed.exprs.ty[both.as_usize()].type_id(), expected);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn package_defined_default_materializes_in_another_file() {
    let declarations =
        TokenCursor::parse_source("#default(IntegerLiteral)\nPackageInteger is in 0...65535\n");
    let consumer = TokenCursor::parse_source("value: 42\n");
    let outputs = transform_package(
        &[(declarations, FileId(10)), (consumer, FileId(11))],
        &TransformCtx::new(),
        PackageTransformOptions::FULL,
    );
    let expr = body(&outputs[1], "value");
    let expected = outputs[0].tags[&typecheck::TagId(Intern::from_ref("PackageInteger"))]
        .resolved_ty
        .type_id();

    assert_eq!(outputs[1].exprs.ty[expr.as_usize()].type_id(), expected);
    assert!(
        outputs.iter().all(|typed| typed.all_flaws().is_empty()),
        "{:?}",
        outputs
            .iter()
            .flat_map(TypedFileAst::all_flaws)
            .collect::<Vec<_>>()
    );
}
