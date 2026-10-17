mod support;

const TYPES: &str = "Length is in 0 to 65535\nLength has finish(self): return\n";

#[test]
fn local_domain_annotations_use_current_arithmetic_proofs() {
    let typed = support::check_no_flaws(&format!(
        "{TYPES}index(offset is in Length, count is in Length, position in 1 to 4) Length:
    when offset >= count then return 0
    when position >= count - offset then return 0
    result is in Length: offset + position
    return result as Length
"
    ));
    let initializer = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .find_map(|(index, kind)| match kind {
            typecheck::TypedExprKind::Bind { name, body, .. } if name.as_str() == "result" => {
                Some((index, body.index()))
            }
            _ => None,
        })
        .unwrap();
    assert_ne!(typed.exprs.ty[initializer.0], typed.exprs.ty[initializer.1]);
}

#[test]
fn local_domain_annotations_do_not_assume_the_target_bounds() {
    for body in [
        "    result is in Length: offset + position\n    return result as Length",
        concat!(
            "    when offset >= count then return 0\n",
            "    when position >= count - offset then return 0\n",
            "    offset :: 65535\n",
            "    result is in Length: offset + position\n",
            "    return result as Length",
        ),
        concat!(
            "    when offset >= count then return 0\n",
            "    when position >= count - offset then return 0\n",
            "    touch(offset)\n",
            "    result is in Length: offset + position\n",
            "    return result as Length",
        ),
    ] {
        let source = format!(
            "{TYPES}touch(value mut is in Length): value :: 65535\n\
             index(offset is in Length, count is in Length, position in 1 to 4) Length:\n{body}\n"
        );
        let typed = support::check_parsed(&source);
        assert!(
            support::flaw_codes(&typed)
                .iter()
                .any(|code| code.starts_with("type-integer-narrowing-")),
            "{source}"
        );
    }
}

#[test]
fn local_domain_annotations_preserve_excluded_values() {
    let declarations = concat!(
        "Nonzero is in 0 to 255 and /= 127\n",
        "Nonzero has finish(self): return\n",
    );
    for (guard, accepted) in [
        ("value < 127", true),
        ("value <= 127", false),
        ("value > 127", true),
    ] {
        let source = format!(
            "{declarations}convert(value in 0 to 255) Nonzero:
    when {guard} then
        narrowed is in Nonzero: value
        return narrowed as Nonzero
    return 0
"
        );
        let typed = support::check_parsed(&source);
        assert_eq!(
            typed.all_flaws().is_empty(),
            accepted,
            "{source}\n{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn borrowed_domain_projection_reads_the_original_integer_type() {
    let typed = support::check_no_flaws(&format!(
        "{TYPES}read(value ref Length) is in Length: return (number is in Length: value)\n"
    ));
    let source = typed
        .exprs
        .kind
        .iter()
        .find_map(|kind| match kind {
            typecheck::TypedExprKind::Bind { name, body, .. } if name.as_str() == "number" => {
                Some(*body)
            }
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        typed.exprs.kind[source.index()],
        typecheck::TypedExprKind::Deref(_)
    ));
    assert!(typed.exprs.ty[source.index()].named_instance().is_some());
}
