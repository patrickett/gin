use super::*;

#[test]
fn record_projection_facts_propagate_from_children_before_parents() {
    let inner = Intern::from_ref("inner");
    let amount = Intern::from_ref("amount");
    let record = ConstValue::Record {
        fields: vec![(
            inner,
            ConstValue::Record {
                fields: vec![(amount, ConstValue::Int(15.into()))].into(),
            },
        )]
        .into(),
    };
    for (field, wrapped) in [amount, Intern::from_ref("missing")]
        .into_iter()
        .flat_map(|field| [(field, false), (field, true)])
    {
        let mut base = Typed::infer(
            crate::const_value::const_value_to_inferred_expr(&record, SpanId::INVALID),
            SpanId::INVALID,
        );
        base.const_value = Some(Box::new(record.clone()));
        let inner_projection = Typed::infer(
            Expr::RecordGet {
                base: Box::new(base),
                field: inner,
            },
            SpanId::INVALID,
        );
        let projection = Typed::infer(
            Expr::RecordGet {
                base: Box::new(inner_projection),
                field,
            },
            SpanId::INVALID,
        );
        let mut expr = if wrapped {
            Typed::infer(Expr::Negate(Box::new(projection)), SpanId::INVALID)
        } else {
            projection
        };
        propagate_record_fields_typed(&mut expr);
        let projection = if wrapped {
            let Expr::Negate(projection) = &expr.value else {
                panic!("wrapper")
            };
            assert_eq!(expr.const_value, None);
            projection.as_ref()
        } else {
            &expr
        };
        let Expr::RecordGet { base, .. } = &projection.value else {
            panic!("projection")
        };
        assert!(matches!(
            base.const_value.as_deref(),
            Some(ConstValue::Record { .. })
        ));
        assert_eq!(
            projection.const_value,
            (field == amount).then(|| Box::new(ConstValue::Int(15.into())))
        );
    }
}

#[test]
fn predicate_constants_respect_callable_parameter_shadowing() {
    use parser::query::SourceParseExt;
    let parsed = "limit: 15\nshadow(limit is in 0 to 255) True thus limit <= 15 or False: return True\nfree(value is in 0 to 255) True thus value <= limit or False: return True\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    let shadow = ast::DefinitionId::named(Intern::from_ref("shadow"));
    let original = ast.defs[&shadow].anonymous_result_alternatives[0]
        .value
        .proposition
        .clone();
    materialize_target_dependent_constants(&mut ast, &CompileTarget::Library);
    assert_eq!(
        ast.defs[&shadow].anonymous_result_alternatives[0]
            .value
            .proposition,
        original
    );
    let free = ast::DefinitionId::named(Intern::from_ref("free"));
    assert!(matches!(
        ast.defs[&free].anonymous_result_alternatives[0].value.proposition.as_ref(),
        Some(ast::ProofProposition::Compare { right: ast::ProofTerm::Value(value), .. })
            if *value == 15.into()
    ));
}

#[test]
fn predicate_constants_respect_record_fields_and_type_parameters() {
    use parser::query::SourceParseExt;
    let parsed = "capacity: 15\nlimit: 3\nexternal: 9\nByte is in 0 to 255\nRegion(limit) has\n    capacity Byte\n    initialized Byte and <= capacity\n    clipped Byte and <= limit\n    bounded Byte and <= external\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    let name = Intern::from_ref("Region");
    let original = ast.tags[&name].value.clone();
    materialize_target_dependent_constants(&mut ast, &CompileTarget::Library);
    let DeclareValue::Has(before) = original else {
        panic!("record declaration")
    };
    let DeclareValue::Has(after) = &ast.tags[&name].value else {
        panic!("record declaration")
    };
    for index in [1, 2] {
        let ast::HasMember::Property(before) = &before[index] else {
            panic!("property")
        };
        let ast::HasMember::Property(after) = &after[index] else {
            panic!("property")
        };
        assert_eq!(before.refinement, after.refinement);
    }
    let ast::HasMember::Property(property) = &after[3] else {
        panic!("property")
    };
    assert!(matches!(
        property.refinement.as_ref(),
        Some(ast::PredicateExpr::Le(ast::NormalExpr::Value(ConstValue::Int(value))))
            if *value == 9.into()
    ));
}

#[test]
fn qualified_empty_tags_keep_their_owner_when_materialized() {
    let name = Intern::from_ref("Stop");
    let value = ConstValue::Tag {
        name,
        qual_path: Some("library.Choice".into()),
        args: Vec::new().into(),
    };
    let Expr::TagCall(call) =
        crate::const_value::const_value_to_inferred_expr(&value, SpanId::INVALID)
    else {
        panic!("qualified constructor became an unqualified tag");
    };
    assert_eq!(call.name, name);
    assert!(call.args.is_empty());
    assert_eq!(
        call.qual_path.unwrap().value.to_qualified_string(),
        "library.Choice"
    );
    assert!(matches!(
        crate::const_value::const_value_to_inferred_expr(
            &ConstValue::Tag {
                name,
                qual_path: None,
                args: Vec::new().into(),
            },
            SpanId::INVALID
        ),
        Expr::AnonymousTag(actual) if actual == name
    ));
}

#[test]
fn forward_constants_resolve_while_cycles_and_runtime_values_remain_unknown() {
    let ast = parser::parse_from_str(
        "first: second\nsecond: third\nthird: 42\ncycle_a: cycle_b\ncycle_b: cycle_a\nruntime(value): return value\n",
    );
    let env = const_env_from_prepared_ast(&ast, None);
    for name in ["first", "second", "third"] {
        assert_eq!(
            env[&Intern::from_ref(name)],
            Some(ConstValue::Int(42.into()))
        );
    }
    for name in ["cycle_a", "cycle_b", "runtime"] {
        assert_eq!(env[&Intern::from_ref(name)], None);
    }
}

#[test]
fn target_constants_and_their_dependents_materialize_without_runtime_binds() {
    let mut ast = parser::parse_from_str(
        "Byte is in 0 to 255\ncopy: width\nwidth: #size(@())\nruntime(value Byte) Byte: return value\n",
    );
    let target =
        CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    materialize_target_dependent_constants(&mut ast, &target);
    for name in ["width", "copy"] {
        let BindValue::Expr(expr) = &ast.defs[&Intern::from_ref(name)].value else {
            panic!("expected expression");
        };
        assert_eq!(expr.const_value, Some(Box::new(ConstValue::Int(8.into()))));
    }
    let BindValue::Body { ret, .. } = &ast.defs[&Intern::from_ref("runtime")].value else {
        panic!("expected expression");
    };
    assert_eq!(ret.values[0].const_value, None);
}

#[test]
fn callable_bodies_are_not_cached_as_argumentless_values() {
    use parser::query::SourceParseExt;
    let parsed =
        "factory(value): return 7\nvalid: factory(1)\nmissing: factory\nextra: factory(1, 2)\n"
            .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let env = const_env_from_prepared_ast(&parsed.ast, None);
    assert_eq!(env[&Intern::from_ref("factory")], None);
    assert_eq!(
        env[&Intern::from_ref("valid")],
        Some(ConstValue::Int(7.into()))
    );
    assert_eq!(env[&Intern::from_ref("missing")], None);
    assert_eq!(env[&Intern::from_ref("extra")], None);
}

#[test]
fn zero_parameter_getters_remain_cached_values_without_accepting_arguments() {
    use parser::query::SourceParseExt;
    let parsed = "getter: 7\ncopy: getter\nextra: getter(1)\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let env = const_env_from_prepared_ast(&parsed.ast, None);
    for name in ["getter", "copy"] {
        assert_eq!(
            env[&Intern::from_ref(name)],
            Some(ConstValue::Int(7.into()))
        );
    }
    assert_eq!(env[&Intern::from_ref("extra")], None);
}

#[test]
fn cached_qualified_getters_reject_arguments() {
    use parser::query::SourceParseExt;
    let parsed = "getter: 7\nextra: space.getter(1)\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    let mut getter = ast
        .defs
        .shift_remove(&ast::DefinitionId::named(Intern::from_ref("getter")))
        .unwrap();
    getter.name = Intern::from_ref("space.getter");
    ast.defs
        .insert(ast::DefinitionId::named(getter.name), getter);
    let env = const_env_from_prepared_ast(&ast, None);
    assert_eq!(
        env[&Intern::from_ref("space.getter")],
        Some(ConstValue::Int(7.into()))
    );
    assert_eq!(env[&Intern::from_ref("extra")], None);
}
