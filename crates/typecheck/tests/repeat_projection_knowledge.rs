mod support;

#[test]
fn repeat_projections_retain_the_seeds_integer_domain() {
    for literal in ["[value & 15, value & 15]", "[value & 15; 2]"] {
        let typed = support::check(&format!(
            "inspect(value is in 0 to 4294967295):\n    values [in 0 to 4294967295; 2]: {literal}\n    return values.1\n"
        ));
        assert!(
            typed.all_flaws().is_empty(),
            "{literal}: {:?}",
            typed.all_flaws()
        );
        let expression = support::body(&typed, "inspect");
        let Some(Some(ast::integer::IntegerKnowledge::Domain(domain))) =
            typed.exprs.integer_knowledge_of(expression)
        else {
            panic!("{literal}: expected integer domain knowledge");
        };
        let hull = domain.storage_hull().unwrap();
        assert_eq!(hull.min(), 0.into(), "{literal}");
        assert_eq!(hull.max(), 15.into(), "{literal}");
    }
}

#[test]
fn replacing_a_repeat_element_discards_its_seed_domain() {
    let typed = support::check(
        "inspect(value is in 0 to 4294967295):\n    values [in 0 to 4294967295; 2]: [value & 15; 2]\n    values.1 :: (value & 255)\n    return values.1\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let expression = support::body(&typed, "inspect");
    let Some(Some(ast::integer::IntegerKnowledge::Domain(domain))) =
        typed.exprs.integer_knowledge_of(expression)
    else {
        panic!("expected replacement integer domain");
    };
    assert!(domain.storage_hull().unwrap().max() >= 255.into());
}

#[test]
fn invalid_repeat_indices_do_not_inherit_the_seed_domain() {
    for (count, index) in [(0, 0), (2, 2)] {
        let typed = support::check(&format!(
            "inspect(value is in 0 to 4294967295):\n    values [in 0 to 4294967295; {count}]: [value & 15; {count}]\n    return values.{index}\n"
        ));
        assert!(
            support::flaw_codes(&typed).contains(&"type-sequence-index-out-of-bounds".to_owned())
        );
        let expression = support::body(&typed, "inspect");
        let Some(Some(ast::integer::IntegerKnowledge::Domain(domain))) =
            typed.exprs.integer_knowledge_of(expression)
        else {
            panic!("expected the element type's domain");
        };
        assert!(domain.storage_hull().unwrap().max() >= 255.into());
    }
}

#[test]
fn inferred_repeats_preserve_the_lowered_seeds_element_type() {
    let typed = support::check(
        r#"Numeric has
    as_num(self ref) is in Self: return (number is in Self: self)
Byte is in 0 to 255
Byte has Numeric
    finish(self):
        return
inspect(value ref Byte): return [value.as_num & 15; 2]
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let expression = support::body(&typed, "inspect");
    let typecheck::TypedExprKind::FixedArrayAlloc { init, .. } =
        &typed.exprs.kind[expression.index()]
    else {
        panic!("expected a repeat");
    };
    let ast::Ty::Array { elem, .. } = &typed.exprs.ty[expression.index()] else {
        panic!("expected an array type");
    };
    assert_eq!(elem.as_ref(), &typed.exprs.ty[init.index()]);
}

#[test]
fn repeat_projection_domains_authorize_only_proven_narrowing() {
    for (update, valid) in [("", true), ("    values.1 :: (value & 255)\n", false)] {
        let typed = support::check(&format!(
            "Nibble is in 0 to 15\nNibble has\n    finish(self):\n        return\nconvert(value is in 0 to 4294967295) Nibble:\n    values [in 0 to 4294967295; 2]: [value & 15; 2]\n{update}    return values.1 as Nibble\n"
        ));
        let codes = support::flaw_codes(&typed);
        if valid {
            assert!(codes.is_empty(), "{codes:?}");
        } else {
            assert!(
                codes
                    .iter()
                    .any(|code| code.starts_with("type-integer-narrowing")),
                "{codes:?}"
            );
        }
    }
}
