mod support;

use support::{body, check, flaw_codes};
use typecheck::TypedExprKind;

#[test]
fn selects_nominal_and_inferred_byte_array_overloads() {
    let typed = check(&format!(
        "{}Length is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nBuffer has\n    finish(self):\n        return\n    length Length\nsize(value own Buffer) Buffer: return value\nsize(bytes [in Byte; count]) Length: return count as Length\nrecord: size(Buffer(length: 7))\ntext: size('abc')\nempty: size('')\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    for (name, target, member) in [
        ("record", "size", 0),
        ("text", "size$array$3", 1),
        ("empty", "size$array$0", 1),
    ] {
        assert!(
            matches!(typed.exprs.kind_of(body(&typed, name)), Some(TypedExprKind::FnCall { target: id, .. }) if id.0.as_str() == target && id.1 == member),
            "{name}: {:?}",
            typed.exprs.kind_of(body(&typed, name))
        );
    }
}

#[test]
fn rejects_ambiguous_and_unmatched_calls() {
    for (source, expected) in [
        (
            "A has\n    value in 0 to 255\nchoose(a A): return 1\nchoose(a A): return 2\nresult: choose(A(value: 1))\n",
            "type-ambiguous-overload",
        ),
        (
            "A has\n    value in 0 to 255\nB has\n    value in 0 to 255\nchoose(a A): return 1\nchoose(b B): return 2\nresult: choose('text')\n",
            "type-no-matching-overload",
        ),
    ] {
        let typed = check(source);
        assert!(
            flaw_codes(&typed).iter().any(|code| code == expected),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn overloads_contextualize_integer_literals_without_guessing_between_domains() {
    let typed = check(&format!(
        "{}Negative is in -128 to -1\nNegative has\n    finish(self):\n        return\nchoose(value Byte) Byte: return value\nchoose(value Negative) Negative: return value\npos: choose(12)\nneg: choose(-2)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let typed = check(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(value Byte) Byte: return value\nchoose(value Word) Word: return value\nresult: choose(12)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-ambiguous-overload"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn named_arguments_are_matched_and_ordered_for_each_signature() {
    let typed = check(&format!(
        "{}Length is in 0 to 65535\nLength has\n    finish(self):\n        return\nchoose(left Byte, right [in Byte; count]) Length: return count as Length\nchoose(left [in Byte; count], right Byte) Length: return count as Length\nresult: choose(right: 'abc', left: 1)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn unique_method_arity_preserves_context_for_generic_target_queries() {
    use parser::query::SourceParseExt;
    use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};

    let parsed = format!("index_bits := #index_bits(@())\nmax_count := (1 << (index_bits - 1)) - 1\n{}Box(element) has\n    finish(self):\n        return\n    value element\n    measure(c is in 0 to max_count) Byte:\n        return Self.measure(c: c, alignment: #alignment(element))\n    measure(c is in 0 to max_count, alignment is in 0 to max_count) Byte: return 0\n", test_fixtures::preludes::BYTE_PRELUDE).parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let target =
        flask::CompileTarget::Concrete(flask::TargetTriple::parse("x86_64-unknown-linux").unwrap());
    let mut asts = vec![parsed.ast];
    let flaws = typecheck::prepare_package_asts(&mut asts, &target);
    assert!(flaws.iter().all(Vec::is_empty), "{flaws:?}");
    let package = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(target),
    );
    for typed in package.typed_asts {
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    }
}

#[test]
fn array_overloads_require_matching_elements_and_consistent_lengths() {
    for call in ["choose('a', 'bc')", "choose([1 as Word], [2 as Word])"] {
        let source = format!(
            "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(left [in Byte; n], right [in Byte; n]) Word: return n as Word\nchoose(left Byte, right Byte) Word: return 0\nresult: {call}\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let typed = check(&source);
        assert!(
            flaw_codes(&typed)
                .iter()
                .any(|code| code == "type-no-matching-overload"),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn array_specialization_does_not_erase_ambiguous_overload_errors() {
    let typed = check(&format!(
        "{}choose(bytes [in Byte; n]) Byte: return 1\nchoose(bytes [in Byte; m]) Byte: return 2\nresult: choose('abc')\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-ambiguous-overload"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn local_values_shadow_overloaded_function_names() {
    let typed = check(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(value Byte) Byte: return value\nchoose(value Word) Word: return value\nidentity(choose Byte) Byte: return choose\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn nominal_integer_arguments_are_not_implicitly_converted() {
    let typed = check(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nOther is in 0 to 255\nchoose(value Byte) Byte: return value\nchoose(value Word) Word: return value\nentry(value Other): return choose(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed)
            .iter()
            .any(|code| code == "type-no-matching-overload"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn selected_overloads_preserve_ownership_checks() {
    let typed = check(&format!(
        "{}Box has\n    finish(self):\n        return\n    value Byte\nchoose(value own Box) Box: return value\nchoose(bytes [in Byte; count]) Byte: return 0\nentry(value own Box) Box:\n    first: choose(value)\n    return choose(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(
        flaw_codes(&typed).iter().any(|code| matches!(
            code.as_str(),
            "type-use-after-move" | "type-use-of-moved-value"
        )),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn overload_groups_resolve_nonconsecutive_definition_ids() {
    use internment::Intern;
    use parser::cursor::TokenCursor;
    use typecheck::transform::{TransformCtx, transform};
    use typecheck::{DefId, FileId};

    let mut ast = TokenCursor::parse_source(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(value Byte) Byte: return value\nchoose(value Word) Word: return value\nresult: choose(7 as Word)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    let name = Intern::from_ref("choose");
    for (old, new) in [(0, 8), (1, 31)] {
        let bind = ast.defs.shift_remove(&DefId::member(name, old)).unwrap();
        ast.defs.insert(DefId::member(name, new), bind);
    }
    assert!(ast.contains_public_symbol(&name));
    for remove_first in [false, true] {
        if remove_first {
            ast.defs.retain(|id, _| *id != DefId::member(name, 8));
        }
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        assert!(
            matches!(typed.exprs.kind_of(body(&typed, "result")), Some(TypedExprKind::FnCall { target, .. }) if *target == DefId::member(name, 31))
        );
    }
}

#[test]
fn array_specializations_preserve_overload_and_parameter_identities() {
    let typed = check(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(bytes [in Byte; count], value Byte) Byte: return 11\nchoose(bytes [in Byte; count], value Word) Byte: return 22\nfirst: choose('abc', 1 as Byte)\nsecond: choose('xyz', 1 as Word)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = |name| match typed.exprs.kind_of(body(&typed, name)) {
        Some(TypedExprKind::FnCall { target, .. }) => *target,
        other => panic!("expected a resolved call, got {other:?}"),
    };
    let first = target("first");
    let second = target("second");
    assert_eq!(first.0, second.0);
    assert_ne!(first, second);
    assert_ne!(
        typed.defs[&first].dependent_binder,
        typed.defs[&second].dependent_binder
    );
}

#[test]
fn overloaded_result_families_have_distinct_owners() {
    let typed = check(&format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(value Byte) True or False: return True\nchoose(value Word) True or False: return False\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let name = internment::Intern::from_ref("choose");
    let group = &typed.overload_groups[&name];
    assert_ne!(
        typed.defs[&group[0]].return_type,
        typed.defs[&group[1]].return_type
    );
}

#[test]
fn hover_uses_the_selected_definition_identity() {
    let source = format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nchoose(value Byte) Byte: return value\nchoose(value Word) Word: return value\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let typed = check(&source);
    let first_line = source
        .lines()
        .position(|line| line.starts_with("choose(value Byte)"))
        .unwrap();
    let second_line = source
        .lines()
        .position(|line| line.starts_with("choose(value Word)"))
        .unwrap();
    let first = typed.hover_at(&source, first_line as u32, 1).unwrap();
    let second = typed.hover_at(&source, second_line as u32, 1).unwrap();
    assert!(first.contains("Byte"), "{first}");
    assert!(second.contains("Word"), "{second}");
    assert!(!second.contains("$"), "{second}");
}
