//! Direct checker API, malformed-node and resource-budget regressions.
//! Source-level identity, claim and rejection cases live in the compiler UI fixtures at
//! `tools/ginc/tests/ui/typecheck/source_check/`.

use typecheck::{Limits, SourceCheckError, SourceType, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn immutable_and_budgeted() {
    let source =
        parser::parse_strict("echo(value in 0 to 16) in 0 to 16:\n  return value\n")
            .unwrap();
    let before = source.clone();
    let first = check_file(&source, Limits::default()).unwrap();
    assert_eq!(first, check_file(&source, Limits::default()).unwrap());
    assert_eq!(source, before);
    assert_eq!(
        check_file(
            &source,
            Limits {
                work_max: 0,
                ..Limits::default()
            }
        ),
        Err(SourceCheckError::Budget)
    );
}

#[test]
fn unit_and_colon_alias() {
    assert_eq!(check("f():\n  return\n"), Ok(()));
    assert_eq!(check("f():\n  alias: 0\n"), Ok(()));
    assert_eq!(
        check("f() in 0 to 0:\n  return\n  return 0\n"),
        Err(SourceCheckError::Unsupported)
    );
    assert_eq!(
        check("f(value in 0 to 16) in 0 to 16 and >= value:\n  alias: value\n  return alias\n"),
        Ok(())
    );
}

#[test]
fn whole_file_rejects_unsupported_items_and_written_features() {
    for text in [
        "Window is 0 to 16\n",
        "f(value: 0) in 0 to 16:\n  return value\n",
        "f(value own Int) in 0 to 16:\n  return value\n",
        "f(value end Int) in 0 to 16:\n  return value\n",
        "f() Int: return 0\n",
    ] {
        assert_eq!(check(text), Err(SourceCheckError::Unsupported), "{text}");
    }
}

#[test]
fn dependent_bounds_are_a_coverage_gap_not_an_invalid_domain() {
    assert_eq!(
        check("f(bound in 0 to 16, value in 0 to bound) in 0 to 0: return 0\n"),
        Err(SourceCheckError::Unsupported)
    );
}

#[test]
fn signed_integer_boundaries_and_hex() {
    let minimum = "-57896044618658097711785492504343953926634992332820282019728792003956564819968";
    let maximum = "57896044618658097711785492504343953926634992332820282019728792003956564819967";
    for literal in [minimum, maximum, "0xF_F", "0XF_F", "-0X80"] {
        let text = format!("f() in {literal} to {literal}: return {literal}\n");
        assert_eq!(check(&text), Ok(()), "{literal}");
    }
    let overflow = "57896044618658097711785492504343953926634992332820282019728792003956564819968";
    assert_eq!(
        check(&format!("f() in 0 to 0: return {overflow}\n")),
        Err(SourceCheckError::Overflow)
    );
}

#[test]
fn exact_work_threshold_is_fail_closed() {
    let source = parser::parse_strict("f() in 0 to 0: return 0\n").unwrap();
    let first_success = (0..1000)
        .find(|budget| {
            check_file(
                &source,
                Limits {
                    work_max: *budget,
                    ..Limits::default()
                },
            )
            .is_ok()
        })
        .unwrap();
    assert!(first_success > 0);
    assert_eq!(
        check_file(
            &source,
            Limits {
                work_max: first_success - 1,
                ..Limits::default()
            }
        ),
        Err(SourceCheckError::Budget)
    );
}

#[test]
fn malformed_manual_nodes_fail_without_panics() {
    let pristine = parser::parse_strict("f() in 0 to 0: return 0\n").unwrap();
    for invalid in [ast::SpanId::INVALID, ast::SpanId::new(99999)] {
        let mut source = pristine.clone();
        source.items[0].span_id = invalid;
        assert_eq!(
            check_file(&source, Limits::default()),
            Err(SourceCheckError::Malformed)
        );
    }
    let mut source = pristine.clone();
    source.items[0].span_id = source.span_table.insert_range(999..1000);
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Malformed)
    );
    let mut source = pristine;
    let ast::Item::Binding(binding) = &mut source.items[0].value else {
        unreachable!()
    };
    binding.value = None;
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::MissingBody)
    );
}

#[test]
fn exposes_verified_domains() {
    let source =
        parser::parse_strict("f(value in 0 to 16 and >= 8) in 8 to 16: return value\n")
            .unwrap();
    let checked = check_file(&source, Limits::default()).unwrap();
    let domain = SourceType::Integer {
        lower: 8.into(),
        upper: 16.into(),
    };
    assert_eq!(checked.functions[0].parameters, [domain]);
    assert_eq!(checked.functions[0].result, domain);
}

#[test]
fn rejects_malformed_literals_and_recovery_nodes() {
    for spelling in ["_", "0x", "1_", "_1", "1__2", "0x_F", "0xGG"] {
        // Replace only the body literal spelling, retaining a manually constructed integer node.
        let mut source = parser::parse_strict("f() in 0 to 0: return 0\n").unwrap();
        let start = source.text.rfind('0').unwrap();
        source.text = format!("{}{}\n", &source.text[..start], spelling).into();
        let literal_span = source
            .span_table
            .insert_range(start..start + spelling.len());
        let item_span = source.span_table.insert_range(0..source.text.len() - 1);
        let body_span = source
            .span_table
            .insert_range(source.text.find("return").unwrap()..source.text.len() - 1);
        source.items[0].span_id = item_span;
        let ast::Item::Binding(binding) = &mut source.items[0].value else {
            unreachable!()
        };
        binding.value.as_mut().unwrap().expression.span_id = body_span;
        let ast::Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value
        else {
            unreachable!()
        };
        block.statements[0].span_id = body_span;
        let ast::Statement::Return(ret) = &mut block.statements[0].value else {
            unreachable!()
        };
        ret.value.as_mut().unwrap().span_id = literal_span;
        assert_eq!(
            check_file(&source, Limits::default()),
            Err(SourceCheckError::Malformed),
            "{spelling}"
        );
    }
    let mut source = parser::parse_strict("f() in 0 to 0: return 0\n").unwrap();
    let ast::Item::Binding(binding) = &mut source.items[0].value else {
        unreachable!()
    };
    let ast::Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value
    else {
        unreachable!()
    };
    let ast::Statement::Return(ret) = &mut block.statements[0].value else {
        unreachable!()
    };
    ret.value.as_mut().unwrap().value = ast::Expr::Missing;
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Unsupported)
    );
}

#[test]
fn invalid_utf8_boundary_is_not_sliced() {
    let mut source = parser::parse_strict("-- hé\nf(): return\n").unwrap();
    let start = source.text.find('é').unwrap();
    source.items[0].span_id = source.span_table.insert_range(start..start + 1);
    assert_eq!(
        check_file(&source, Limits::default()),
        Err(SourceCheckError::Malformed)
    );
}
