mod support;

use typecheck::TypedExprKind;
use typecheck::ty::Ty;

#[test]
fn inferred_repeats_retain_one_seed_and_a_count() {
    for count in [0, 3, 4096] {
        let typed = support::check(&format!(
            "{}values(value Byte): return [value; {count}]\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let expression = support::body(&typed, "values");
        let TypedExprKind::FixedArrayAlloc { init, size } = &typed.exprs.kind[expression.index()]
        else {
            panic!("inferred repeat must retain its seed and count");
        };
        assert_eq!(size.as_const_int(), Some(count));
        let Ty::Array { elem, size } = &typed.exprs.ty[expression.index()] else {
            panic!("inferred repeat must retain its structural array type");
        };
        assert_eq!(elem.as_ref(), &typed.exprs.ty[init.index()]);
        assert_eq!(size.as_const_int(), Some(count));
        assert!(matches!(
            &typed.exprs.facts.const_facts.sequence_literal[expression.index()].as_deref(),
            Some(typecheck::typed::TypedSequenceLiteral::Repeat { seed, count: length })
                if seed == init && length.as_const_int() == Some(count)
        ));
    }
}

#[test]
fn compile_time_empty_repeat_still_evaluates_its_seed() {
    let typed = support::check(&format!(
        "{}replace(previous Byte, next Byte) Byte: return next\nseed(state mut Byte) Byte:\n    state:: replace(deref state, 2)\n    return 0\nconsume(values [Byte; 0]): return\nresult Byte:\n    state Byte: 1\n    values: [seed(state); 0]\n    consume(values)\n    return state\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    let value = typecheck::eval::Evaluator::new(
        typed.resolved_program(),
        typecheck::eval::CompileTimeHost::new(&target),
    )
    .call(
        typecheck::DefId::named(internment::Intern::from_ref("result")),
        &[],
    )
    .unwrap();
    assert_eq!(value, ast::ConstValue::Int(2.into()));
}
