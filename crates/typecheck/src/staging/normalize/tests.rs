use super::*;
use parser::query::SourceParseExt;

fn program(source: &str) -> TypedFileAst {
    let mut parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    crate::prepare_parse_ast(&mut parsed.ast, &flask::CompileTarget::Library);
    crate::transform::transform_file(parsed.ast, crate::FileId(7))
}

fn root(typed: &TypedFileAst, name: &str) -> ExprId {
    match typed.defs[&DefId::named(Intern::from_ref(name))].body {
        BindBody::Expr(root) => root,
        ref other => panic!("unexpected body: {other:?}"),
    }
}

#[test]
fn constant_answers_do_not_demand_expression_ownership() {
    let typed = program("value: 7\n");
    let normalizer = Normalizer::new(typed.resolved_program());
    for _ in 0..4 {
        assert_eq!(
            normalizer.normalize(root(&typed, "value"), &DepSubst::new()),
            NormalizeResult::Value(ConstValue::Int(7.into()))
        );
    }
    assert!(normalizer.owners.get().is_none());
}

#[test]
fn shared_ownership_preserves_binders_and_distinct_substitutions() {
    let typed = program(
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\nfirst(x Word) Word: return x\nsecond(x Word) Word: return x\n",
    );
    let normalizer = Normalizer::new(typed.resolved_program());
    for name in ["first", "second", "first"] {
        let bind = &typed.defs[&DefId::named(Intern::from_ref(name))];
        assert_eq!(
            normalizer.normalize(root(&typed, name), &DepSubst::new()),
            NormalizeResult::Residual(NormalExpr::Param(DependentArgId::new(
                bind.dependent_binder,
                0
            )))
        );
        for value in [3, 7, 3] {
            let mut substitutions = DepSubst::new();
            substitutions.consts.insert(
                Intern::from_ref("x"),
                DependentNormalExpr::Value(ConstValue::Int(value.into())),
            );
            assert_eq!(
                normalizer.normalize(root(&typed, name), &substitutions),
                NormalizeResult::Value(ConstValue::Int(value.into()))
            );
        }
    }
    let owners = normalizer.owners.get().unwrap();
    assert_ne!(
        owners[&root(&typed, "first")],
        owners[&root(&typed, "second")]
    );
}

#[test]
fn demanded_batches_match_eager_ownership_for_resolved_arenas() {
    for source in [
        "value: 7\nother: value\n",
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\nfirst(x Word) Word: return x\nsecond(x Word) Word: return first(x)\n",
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\nnested(x Word) Word:\n    local: x\n    return local\n",
        "Word is in 0 to 255\nWord has\n    finish(self):\n        return\ncycle(x Word) Word: return cycle(x)\n",
    ] {
        let mut typed = program(source);
        typed.exprs.facts.const_facts.const_value.fill(None);
        let normalizer = Normalizer::new(typed.resolved_program());
        let eager = Normalizer::new(typed.resolved_program());
        eager
            .owners
            .set(ExpressionOwners::collect(typed.resolved_program()))
            .unwrap();
        for value in [None, Some(3), Some(7), None] {
            let mut substitutions = DepSubst::new();
            if let Some(value) = value {
                substitutions.consts.insert(
                    Intern::from_ref("x"),
                    DependentNormalExpr::Value(ConstValue::Int(value.into())),
                );
            }
            for index in 0..typed.exprs.kind.len() {
                let id = ExprId(index as u32);
                assert_eq!(
                    normalizer.normalize(id, &substitutions),
                    eager.normalize(id, &substitutions),
                    "{source:?} expression {index}"
                );
            }
        }
    }
}

#[test]
fn constant_record_fields_normalize_with_and_without_cached_values() {
    let mut typed = program(
        "Byte is in 0 to 255\nByte has finish(self): return\nPair has\n    first Byte, second Byte\npair Pair := Pair(first: 7, second: 9)\nfirst: pair.first\nsecond: pair.second\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for clear_cache in [false, true] {
        if clear_cache {
            typed.exprs.facts.const_facts.const_value.fill(None);
        }
        for (name, expected) in [("first", 7), ("second", 9)] {
            assert_eq!(
                normalize(&typed, root(&typed, name), &DepSubst::new()),
                NormalizeResult::Value(ConstValue::Int(expected.into()))
            );
        }
    }
}

#[test]
fn constant_record_addresses_remain_runtime_values() {
    let typed = program(
        "Byte is in 0 to 255\nByte has finish(self): return\nSlice has\n    ptr @Byte, len Byte\ntext Slice := Slice(ptr: (@'é🦀') as @Byte, len: 6)\naddress @Byte: text.ptr\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for name in ["text", "address"] {
        assert!(matches!(
            normalize(&typed, root(&typed, name), &DepSubst::new()),
            NormalizeResult::RuntimeDependency(_)
        ));
    }
}
