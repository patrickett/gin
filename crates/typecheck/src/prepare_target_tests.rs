use super::*;

#[test]
fn forward_constants_resolve_while_cycles_and_runtime_values_remain_unknown() {
    let ast = parser::parse_from_str(
        "first: second\nsecond: third\nthird: 42\ncycle_a: cycle_b\ncycle_b: cycle_a\nruntime(value): value\n",
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
        "Byte is in 0 to 255\ncopy: width\nwidth: #size(@())\nruntime(value Byte) Byte: value\n",
    );
    let target =
        CompileTarget::Concrete(flask::TargetTriple::parse("aarch64-apple-darwin").unwrap());
    materialize_target_dependent_constants(&mut ast, &target);
    for name in ["width", "copy"] {
        let BindValue::Expr(expr) = &ast.defs[&Intern::from_ref(name)].value else {
            panic!("expected expression");
        };
        assert_eq!(expr.const_value, Some(ConstValue::Int(8.into())));
    }
    let BindValue::Expr(runtime) = &ast.defs[&Intern::from_ref("runtime")].value else {
        panic!("expected expression");
    };
    assert_eq!(runtime.const_value, None);
}
