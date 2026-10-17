use parser::query::SourceParseExt;
use typecheck::{FileId, prepare::prepare_parse_ast, transform::transform_file};

#[test]
fn module_qualification_preserves_receiver_parameter_names() {
    let parsed = "limit: 15\nCount is in 0 to 255\nContainer(limit Count) has\n    check(value Count) True thus value <= limit or False: return True\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let original = parsed
        .ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "check")
        .unwrap()
        .anonymous_result_alternatives[0]
        .value
        .proposition
        .clone();
    let mut ast = parsed.ast.qualify_module_defs("library");
    let check = ast
        .defs
        .iter()
        .find_map(|(name, bind)| {
            (name.0.as_str().ends_with(".check") && bind.receiver_type_surface().is_some())
                .then_some(*name)
        })
        .expect("qualified method");
    assert_eq!(
        ast.defs[&check].anonymous_result_alternatives[0]
            .value
            .proposition,
        original
    );
    typecheck::prepare_target::materialize_target_dependent_constants(
        &mut ast,
        &flask::CompileTarget::Library,
    );
    assert_eq!(
        ast.defs[&check].anonymous_result_alternatives[0]
            .value
            .proposition,
        original
    );
}

#[test]
fn concrete_receiver_arguments_do_not_shadow_global_contract_constants() {
    let parsed = "limit: 15\nCount is in 0 to 255\nConcrete(limit Count) has\n    marker Count\nConcrete(3).inspect(value Count) True thus value <= limit or False: return True\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    typecheck::prepare_target::materialize_target_dependent_constants(
        &mut ast,
        &flask::CompileTarget::Library,
    );
    let method = ast
        .defs
        .values()
        .find(|bind| bind.name.as_str() == "inspect")
        .expect("concrete method");
    assert!(matches!(
        method.anonymous_result_alternatives[0].value.proposition.as_ref(),
        Some(ast::ProofProposition::Compare { right: ast::ProofTerm::Value(value), .. }) if *value == 15.into()
    ));
}

#[test]
fn materialized_methods_keep_their_receiver_parameter_scope() {
    use internment::Intern;
    let parsed = "limit: 15\nexternal: 9\nCount is in 0 to 255\nContainer(limit Count) has\n    check(value Count) True thus value <= limit or False: return True\n    free(value Count) True thus value <= external or False: return True\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    let check = ast
        .defs
        .iter()
        .find_map(|(key, bind)| {
            (bind.name.as_str() == "check" && bind.receiver_type_surface().is_some())
                .then_some(*key)
        })
        .expect("materialized method");
    let original = ast.defs[&check].anonymous_result_alternatives[0]
        .value
        .proposition
        .clone();
    typecheck::prepare_target::materialize_target_dependent_constants(
        &mut ast,
        &flask::CompileTarget::Library,
    );
    assert_eq!(
        ast.defs[&check].anonymous_result_alternatives[0]
            .value
            .proposition,
        original
    );
    let free = ast
        .defs
        .values()
        .find(|bind| {
            bind.name == Intern::from_ref("free") && bind.receiver_type_surface().is_some()
        })
        .expect("free method");
    assert!(matches!(
        free.anonymous_result_alternatives[0].value.proposition.as_ref(),
        Some(ast::ProofProposition::Compare { right: ast::ProofTerm::Value(value), .. }) if *value == 9.into()
    ));
}

#[test]
fn record_refinements_use_sibling_fields_instead_of_same_named_globals() {
    for initialized in [0, 1, 2, 15] {
        let source = format!(
            "capacity: 15\nCount is in 0 to 255\nCount has finish(self): return\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    finish(self): return\nmake Region: Region(1, {initialized})\n"
        );
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut ast = parsed.ast;
        assert!(prepare_parse_ast(&mut ast, &flask::CompileTarget::Library).is_empty());
        let typed = transform_file(ast, FileId(0));
        let flaws = typed.all_flaws();
        if initialized <= 1 {
            assert!(flaws.is_empty(), "{source}: {flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "dep-refinement-failed"),
                "invalid field refinement accepted: {source}: {flaws:?}"
            );
        }
    }
}

#[test]
fn preparation_preserves_runtime_contract_names_and_global_constants() {
    for (source, valid) in [
        (
            "limit: 15\nshadow(limit is in 0 to 255) True thus limit <= 15 or False: return True\n",
            false,
        ),
        (
            "limit: 15\nshadow(limit is in 0 to 255) True thus limit <= 15 or False:\n    when limit <= 15 then\n        return True\n    return False\n",
            true,
        ),
        (
            "limit: 15\nfree(value is in 0 to 255) True thus value <= limit or False:\n    when value <= 15 then\n        return True\n    return False\n",
            true,
        ),
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut ast = parsed.ast;
        assert!(prepare_parse_ast(&mut ast, &flask::CompileTarget::Library).is_empty());
        let typed = transform_file(ast, FileId(0));
        let flaws = typed.all_flaws();
        if valid {
            assert!(flaws.is_empty(), "{source}: {flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-result-proof-unproven"),
                "unproved runtime contract accepted: {flaws:?}"
            );
        }
    }
}

#[test]
fn method_propositions_fold_globals_without_replacing_parameters() {
    use ast::declare::DeclareValue;
    use flask::CompileTarget;
    use internment::Intern;
    use typecheck::prepare_target::materialize_target_dependent_constants;
    let parsed = "limit: 15\nUnit has\n    free(value is in 0 to 255) True thus value <= limit or False: return True\n    shadow(limit is in 0 to 255) True thus limit <= 15 or False: return True\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast;
    let name = Intern::from_ref("Unit");
    let DeclareValue::Has(members) = &ast.tags[&name].value else {
        panic!("record declaration")
    };
    let ast::HasMember::Function(shadow) = &members[1] else {
        panic!("method")
    };
    let original = shadow.anonymous_result_alternatives[0]
        .value
        .proposition
        .clone();
    materialize_target_dependent_constants(&mut ast, &CompileTarget::Library);
    let DeclareValue::Has(members) = &ast.tags[&name].value else {
        panic!("record declaration")
    };
    let ast::HasMember::Function(free) = &members[0] else {
        panic!("method")
    };
    assert!(matches!(
        free.anonymous_result_alternatives[0].value.proposition.as_ref(),
        Some(ast::ProofProposition::Compare { right: ast::ProofTerm::Value(value), .. })
            if *value == 15.into()
    ));
    let ast::HasMember::Function(shadow) = &members[1] else {
        panic!("method")
    };
    assert_eq!(
        shadow.anonymous_result_alternatives[0].value.proposition,
        original
    );
}
