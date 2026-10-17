use codegen::{CodegenContext, CodegenSourceMap, NativeCompiler, NativeEngine};
use internment::Intern;
use typecheck::transform::{TransformCtx, transform};

fn prefix() -> String {
    let fixture = test_fixtures::emission::increment_source(&test_fixtures::host_target_triple())
        .replace(
            "Word has\n",
            "Word has\n    copy(self ref) Self: return deref self\n",
        );
    format!(
        "{fixture}\nreplace(previous Word, next Word) Word: return next\nbump(state mut Word) Word:\n    state:: replace(deref state, increment(Word.copy(state)))\n    return Word.copy(state)\ntouch(state mut Word) Word:\n    bump(state)\n    return 7\n"
    )
}

fn engine(source: &str, known: Option<(&str, &ast::ConstValue)>) -> NativeEngine {
    let parsed = parser::parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse(&test_fixtures::host_target_triple()).unwrap(),
    );
    let mut scope = TransformCtx::new();
    scope.compile_target = target.clone();
    let mut typed = transform(&parsed, typecheck::FileId(0), &scope);
    assert!(
        typed.all_flaws().is_empty(),
        "{source}: {:?}",
        typed.all_flaws()
    );
    if let Some((callable, value)) = known {
        let bind = &typed.defs[&typecheck::DefId::named(Intern::from_ref(callable))];
        let returned = match bind.body {
            typecheck::BindBody::Expr(id) | typecheck::BindBody::Body { ret: Some(id), .. } => id,
            _ => panic!("return expression"),
        };
        assert!(matches!(
            typed.exprs.kind[returned.as_usize()],
            typecheck::TypedExprKind::When(_)
        ));
        typed.exprs.facts.const_facts.const_value[returned.as_usize()] = Some(value.clone());
    }
    let context = NativeCompiler::create_context();
    let (module, diagnostics) = CodegenContext::build_module_from_resolved_program_for_target(
        &context,
        typed.resolved_program(),
        CodegenSourceMap::new("when_effects.gin", source, &typed.span_table),
        &target,
    );
    assert!(diagnostics.is_empty(), "{source}: {diagnostics:?}");
    NativeEngine::new_with_export_abi(module.unwrap(), "select").unwrap()
}

#[allow(unsafe_code)]
fn check(engine: &NativeEngine, expected: u64) {
    for initial in [41u64, 42] {
        let mut state = initial;
        let mut pointer = &mut state as *mut u64;
        let mut result = 0u64;
        unsafe {
            engine.invoke_packed(&mut [
                (&mut pointer as *mut *mut u64).cast(),
                (&mut result as *mut u64).cast(),
            ])
        }
        .unwrap();
        assert_eq!(result, expected);
        assert_eq!(state, initial + 1);
    }
}

#[test]
fn a_known_when_result_preserves_selector_and_branch_effects() {
    for expression in [
        "when bump(state) is 42 then 7 else 7",
        "when Word.copy(state) is 42 then touch(state) else touch(state)",
    ] {
        let source = format!(
            "{}select(state mut Word) Word: return {expression}\n",
            prefix()
        );
        let value = ast::ConstValue::Int(7.into());
        for known in [None, Some(("select", &value))] {
            check(&engine(&source, known), 7);
        }
    }
}

#[test]
fn known_aggregate_when_results_preserve_payloads_and_selector_effects() {
    use ast::ConstValue;
    let seven = ConstValue::Int(7.into());
    for (body, value, expected) in [
        (
            "Box has\n    value Word\n    finish(self):\n        return\nchoose(state mut Word) Box: return when bump(state) is 42 then Box(value: 7) else Box(value: 7)\nselect(state mut Word) Word:\n    chosen: choose(state)\n    return chosen.value\n",
            ConstValue::Tag {
                name: Intern::from_ref("Box"),
                qual_path: None,
                args: vec![seven.clone()].into(),
            },
            7,
        ),
        (
            "choose(state mut Word) [Word; 2]: return when bump(state) is 42 then [7 as Word, 8 as Word] else [7 as Word, 8 as Word]\nselect(state mut Word) Word:\n    chosen: choose(state)\n    return chosen.[1]\n",
            ConstValue::FixedArray(vec![seven.clone(), ConstValue::Int(8.into())].into()),
            8,
        ),
        (
            "Maybe(element) is Some(element) or None\nMaybe(element) has\n    finish(self):\n        return\nchoose(state mut Word) Maybe(Word): return when bump(state) is 42 then Maybe.Some(7) else Maybe.Some(7)\nselect(state mut Word) Word:\n    chosen: choose(state)\n    when chosen is Some(value) then\n        return value\n    return 0\n",
            ConstValue::Tag {
                name: Intern::from_ref("Some"),
                qual_path: Some("Maybe".into()),
                args: vec![seven.clone()].into(),
            },
            7,
        ),
    ] {
        let source = format!("{}{body}", prefix());
        for known in [None, Some(("choose", &value))] {
            check(&engine(&source, known), expected);
        }
    }
}

#[test]
fn nested_fallthrough_evaluates_the_outer_selector_once() {
    let source = format!(
        "{}select(state mut Word) Word: return when bump(state) is\n    42 and Word.copy(state) is\n        0 then 1\n        else 7\n    else 7\n",
        prefix()
    );
    check(&engine(&source, None), 7);
}
