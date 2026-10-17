mod support;

use ast::ty::Ty;
use internment::Intern;
use support::{check, flaw_codes};
use typecheck::{DefId, TypedFileAst};

fn result_type(typed: &TypedFileAst) -> &Ty {
    &typed.defs[&DefId::named(Intern::from_ref("choose"))].return_type
}

fn assert_domain(ty: &Ty, values: &[i128], excluded: &[i128], width: u32) {
    let validity = ty.anonymous_integer_validity().expect("anonymous result");
    for value in values {
        assert!(
            validity.domain().contains((*value).into()),
            "{ty:?}: {value}"
        );
    }
    for value in excluded {
        assert!(
            !validity.domain().contains((*value).into()),
            "{ty:?}: {value}"
        );
    }
    assert_eq!(
        validity
            .resolve_runtime_representation(
                &ast::integer::IntegerRepresentationRule::InferFromValidity
            )
            .unwrap()
            .width()
            .get(),
        width
    );
}

#[test]
fn early_integer_returns_join_the_complete_value_domains() {
    let typed =
        check("choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256\n    return 0\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_domain(result_type(&typed), &[0, 256], &[1, 255, 257], 9);
}

#[test]
fn nested_and_loop_returns_contribute_to_the_inferred_domain() {
    for body in [
        "    when flag = 1 then\n        when flag = 1 then\n            return -129\n        return 256\n    return 0",
        "    for index in 0 to 1\n        when index = flag then\n            when flag = 1 then\n                return -129\n            return 256\n    return 0",
    ] {
        let typed = check(&format!("choose(flag is in 0 to 2):\n{body}\n"));
        assert!(
            typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
        assert_domain(result_type(&typed), &[-129, 0, 256], &[-130, 1, 257], 10);
    }
}

#[test]
fn return_inference_uses_guarded_occurrence_domains() {
    let typed = check(
        "choose(value is in 0 to 511):\n    when value is in 0 to 255 then\n        return value\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_domain(result_type(&typed), &[0, 1, 254, 255], &[256, 511], 8);
}

#[test]
fn early_return_rows_join_each_integer_column() {
    let typed = check(
        "choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256, -129\n    return 0, 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let Ty::Sequence(fields) = result_type(&typed) else {
        panic!("expected result row")
    };
    assert_eq!(fields.len(), 2);
    assert_domain(&fields[0].ty, &[0, 256], &[1], 9);
    assert_domain(&fields[1].ty, &[-129, 0], &[-1], 9);
}

#[test]
fn inferred_return_domains_reject_unrepresentable_signed_unsigned_combinations() {
    let typed = check(
        "choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 340282366920938463463374607431768211455\n    return -1\n",
    );
    assert!(
        flaw_codes(&typed).contains(&"type-integer-runtime-width-unsupported".to_string()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn forward_callers_receive_the_complete_inferred_return_type() {
    let typed = check(
        "forward(flag is in 0 to 1): return choose(flag)\nchoose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_domain(
        &typed.defs[&DefId::named(Intern::from_ref("forward"))].return_type,
        &[0, 256],
        &[1],
        9,
    );
}

#[test]
fn inferred_returns_include_values_rebound_after_an_early_exit() {
    let typed = check(
        "choose(flag is in 0 to 1):\n    value: 0\n    when flag = 1 then\n        return value\n    value:: 65536\n    return value\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_domain(result_type(&typed), &[0, 65536], &[1, 65535], 17);
}

#[test]
fn inferred_returns_preserve_a_named_integer_type_with_a_literal_fallback() {
    let typed = check(
        "Byte is in 0 to 255\nByte has finish(self): return\nchoose(flag is in 0 to 1, value Byte):\n    when flag = 1 then\n        return value\n    value.finish\n    return 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        matches!(result_type(&typed), Ty::Named {name, ..} if name.as_str() == "Byte"),
        "{:?}",
        result_type(&typed)
    );
}

#[test]
fn inferred_named_integer_returns_reject_invalid_fallbacks() {
    for fallback in ["256", "-1"] {
        let typed = check(&format!(
            "Byte is in 0 to 255\nByte has finish(self): return\nchoose(flag is in 0 to 1, value Byte):\n    when flag = 1 then\n        return value\n    value.finish\n    return {fallback}\n"
        ));
        assert!(
            flaw_codes(&typed).contains(&"type-integer-narrowing-failed".to_string()),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn inferred_integer_returns_reject_distinct_nominal_types() {
    let typed = check(
        "Byte is in 0 to 255\nByte has finish(self): return\nOtherByte is in 0 to 255\nOtherByte has finish(self): return\nchoose(flag is in 0 to 1, value Byte, other OtherByte):\n    when flag = 1 then\n        other.finish\n        return value\n    value.finish\n    return other\n",
    );
    assert!(
        flaw_codes(&typed).contains(&"type-return-mismatch".to_string()),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_callable_inference_includes_early_integer_returns() {
    let typed = check(
        "choose(flag is in 0 to 1):\n    local():\n        when flag = 1 then\n            return 256\n        return 0\n    return local()\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_domain(result_type(&typed), &[0, 256], &[1], 9);
}

#[test]
fn integer_return_inference_rejects_a_bare_final_return() {
    let typed =
        check("choose(flag is in 0 to 1):\n    when flag = 1 then\n        return 256\n    return\n");
    assert!(
        flaw_codes(&typed).contains(&"type-return-mismatch".to_string()),
        "{:?}",
        typed.all_flaws()
    );
}
