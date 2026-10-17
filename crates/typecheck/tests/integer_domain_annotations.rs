mod support;

use internment::Intern;
use parser::query::SourceParseExt;
use support::check_parsed;
use typecheck::{DefId, FileId, transform::transform_file};

#[test]
fn self_domain_uses_primitive_operations_inside_nominal_operators() {
    let typed = check_parsed(
        r#"
Word is in -128 to 127
Word has
    #infix('-')
    subtract(self, rhs Self) Self:
        left is in Self: self
        right is in Self: rhs
        biased: left - right + 128
        bits is in 0 to 255: biased & 255
        return (bits - 128) as Self
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(!typed.exprs.kind.iter().any(|kind| matches!(kind,
        typecheck::TypedExprKind::FnCall { target, .. } if target.0.as_str() == "Word.subtract")));
}

#[test]
fn named_domain_resolves_parameter_and_return_annotations() {
    let typed = check_parsed(&format!(
        "{}identity(value is in Byte) is in Byte: return value\nanswer: identity(42)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bind = &typed.defs[&DefId::named(Intern::from_ref("identity"))];
    assert!(bind.params[0].1.is_int());
    assert_eq!(bind.params[0].1, bind.return_type);
    let domain = bind
        .return_type
        .anonymous_integer_validity()
        .unwrap()
        .domain();
    assert!(domain.contains(255.into()));
    assert!(!domain.contains(256.into()));
}

#[test]
fn non_integer_domains_are_rejected() {
    for source in [
        "Record has\n    field\n    finish(self):\n        return\nbad(value is in Record): return value\n",
        "Record has\n    field\n    finish(self):\n        return\nbad:\n    value is in Record: 0\n    return value\n",
        "Record has\n    field\n    finish(self):\n        return\nbad is in Record: 0\n",
        "Record has\n    field\n    finish(self):\n        return\nbad(value Record) is in Record: return 0\n",
    ] {
        let typed = check_parsed(source);
        let flaws = typed.all_flaws();
        let domain_flaws: Vec<_> = flaws
            .iter()
            .filter(|(_, d)| d.code.slug() == "type-non-integer-domain")
            .collect();
        assert_eq!(domain_flaws.len(), 1, "{flaws:?}");
        let (_, flaw) = domain_flaws[0];
        assert_eq!(flaw.category, diagnostic::Category::Flaw);
        assert_eq!(
            flaw.message,
            "`is in` requires an integer domain, but `Record` is a record type"
        );
        assert!(
            flaw.help
                .as_ref()
                .unwrap()
                .contains("records have no integer domain")
        );
        assert_eq!(flaw.span.start(), source.rfind("Record").unwrap());
        assert_eq!(flaw.span.end(), flaw.span.start() + "Record".len());
    }
}

#[test]
fn trait_parameter_projection_rejects_record_implementations() {
    let source = "Word is in 0 to 9\nWord has\n    finish(self):\n        return\nInspect has\n    inspect(self) Word\nRecord has\n    field Word\n    finish(self):\n        return\nRecord has Inspect\n    finish(self):\n        return\n    inspect(self is in Self): return 0\n";
    let typed = check_parsed(source);
    let flaws = typed.all_flaws();
    let (_, flaw) = flaws
        .iter()
        .find(|(_, d)| d.code.slug() == "type-non-integer-domain")
        .unwrap_or_else(|| panic!("{flaws:?}"));
    assert_eq!(flaw.category, diagnostic::Category::Flaw);
    assert_eq!(
        flaw.message,
        "`is in` requires an integer domain, but `Self` resolves to record type `Record`"
    );
    assert_eq!(flaw.span.start(), source.rfind("Self").unwrap());
    assert_eq!(flaw.span.end(), flaw.span.start() + "Self".len());
}

#[test]
fn domain_projection_preserves_excluded_values() {
    let typed = check_parsed(
        "Nonzero is in -8 to 8 and /= 0\nNonzero has\n    finish(self):\n        return\nidentity(value is in Nonzero) is in Nonzero: return value\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let ty = &typed.defs[&DefId::named(Intern::from_ref("identity"))].return_type;
    let domain = ty.anonymous_integer_validity().unwrap().domain();
    assert!(domain.contains((-8).into()));
    assert!(domain.contains(8.into()));
    assert!(!domain.contains(0.into()));
}

#[test]
fn projection_rejects_values_outside_the_named_domain() {
    for source in [
        format!(
            "{}bad:\n    value is in Byte: 256\n    return value\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        "Nonzero is in -8 to 8 and /= 0\nNonzero has\n    finish(self):\n        return\nbad:\n    value is in Nonzero: 0\n    return value\n".to_string(),
    ] {
        let typed = check_parsed(&source);
        assert!(!typed.all_flaws().is_empty(), "invalid projection accepted");
    }
}

#[test]
fn projection_accepts_nominal_values_in_the_same_domain() {
    let typed = check_parsed(&format!(
        "{}identity(value is in Byte) is in Byte: return value\nextract(value Byte) is in Byte: return identity(value)\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn domain_projection_works_for_observed_integer_values() {
    let typed = check_parsed(
        "Word is in -128 to 127\nWord has\n    finish(self):\n        return\n    copy(value ref Self) Self:\n        raw is in Self: value\n        return raw as Self\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn imported_integer_domains_use_the_type_registry() {
    let definition =
        check_parsed("Octet is in 0 to 255\nOctet has\n    finish(self):\n        return\n");
    let ctx = typecheck::transform::TransformCtx::from_typed_asts(&[&definition]);
    let parsed = "use numbers Octet\nidentity(value is in Octet) is in Octet: return value\n"
        .parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform(&parsed.ast, FileId(1), &ctx);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed.defs[&DefId::named(Intern::from_ref("identity"))]
            .return_type
            .is_int()
    );
}

#[test]
fn top_level_and_return_domain_annotations_reject_invalid_values() {
    for source in [
        format!(
            "{}value is in Byte: 256\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}value(input is in Byte) is in Byte: return 256\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
        format!(
            "{}value(input is in Byte) is in Byte:\n    return 256\n",
            test_fixtures::preludes::BYTE_PRELUDE
        ),
    ] {
        assert!(
            !check_parsed(&source).all_flaws().is_empty(),
            "invalid projection accepted: {source}"
        );
    }
}

#[test]
fn trait_parameter_projections_keep_nominal_dispatch_and_primitive_bodies() {
    let typed = check_parsed(
        r#"
Mul has
    #infix('*'), #inline
    multiply(self, rhs Self) Self
Bit is in 0 to 1
Bit has Mul
    multiply(self is in Self, target is in Self): return self * target
Other is in 0 to 1
Other has Mul
    multiply(self is in Self, rhs is in Self): return self * rhs
product(left Bit, right Bit) Bit: return left * right
method(left Bit, right Bit) Bit: return (left).multiply(right)
other(left Other, right Other) Other: return left * right
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bit = &typed.defs[&DefId::named(Intern::from_ref("Bit.multiply"))];
    assert_eq!(bit.params[0].1, bit.return_type);
    assert_eq!(bit.params[1].1, bit.return_type);
    assert!(matches!(bit.return_type, ast::Ty::Named { .. }));
    for (name, count) in [("Bit.multiply", 2), ("Other.multiply", 1)] {
        assert_eq!(
            typed
                .exprs
                .kind
                .iter()
                .filter(|kind| matches!(kind,
            typecheck::TypedExprKind::FnCall { target, .. } if target.0.as_str() == name))
                .count(),
            count
        );
    }
}

#[test]
fn projected_trait_returns_require_validity_proof() {
    for body in [
        ": self * rhs",
        ":\n        result: self * rhs\n        return result",
        ":\n        when self is 9 then\n            return self * rhs\n        return self",
    ] {
        let typed = check_parsed(&format!(
            "Mul has\n    #infix('*')\n    multiply(self, rhs Self) Self\nDigit is in 0 to 9\nDigit has Mul\n    multiply(self is in Self, rhs is in Self){body}\n"
        ));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, d)| d.code.slug().starts_with("type-integer-narrowing-")),
            "out-of-range product accepted: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn projected_trait_parameters_must_match_nominal_contract() {
    let typed = check_parsed(
        r#"
Mul has
    #infix('*')
    multiply(self, rhs Self) Self
Bit is in 0 to 1
Other is in 0 to 1
Bit has Mul
    multiply(self is in Self, rhs is in Other): return self * rhs
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, d)| d.code.slug() == "type-trait-method-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn projected_trait_methods_reject_other_nominal_arguments() {
    let typed = check_parsed(
        r#"
Mul has
    #infix('*')
    multiply(self, rhs Self) Self
Bit is in 0 to 1
Other is in 0 to 1
Bit has Mul
    multiply(self is in Self, rhs is in Self): return self * rhs
bad(left Bit, right Other) Bit: return left * right
"#,
    );
    assert!(
        !typed.all_flaws().is_empty(),
        "mixed nominal operands accepted"
    );
}

#[test]
fn projected_trait_block_return_can_restore_nominal_type() {
    let typed = check_parsed(
        r#"
Mul has
    #infix('*')
    multiply(self, rhs Self) Self
Bit is in 0 to 1
Bit has Mul
    multiply(self is in Self, rhs is in Self):
        result: self * rhs
        return result
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn associated_functions_with_named_domain_locals_remain_callable() {
    let typed = check_parsed(&format!(
        "{}Encoder has\n    encode(value Word) Word:\n        raw is in Word: value\n        return raw as Word\nanswer: Encoder.encode(42)\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed
            .defs
            .contains_key(&DefId::named(Intern::from_ref("Encoder.encode")))
    );
}

#[test]
fn trait_implementations_with_named_domain_locals_remain_present() {
    let typed = check_parsed(&format!(
        "{}Encode has\n    encode(self, value Word) Word\nEncoder has Encode\n    encode(self, value Word) Word:\n        raw is in Word: value\n        return raw as Word\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn domain_projection_of_parameter_ignores_same_named_function() {
    let parsed = "Octet is in 0 to 255\nOctet has\n    finish(self):\n        return\nRecord has\n    item Octet\n    finish(self):\n        return\nbuffer(size Octet) Octet: return size\nextract(buffer ref Record) Octet:\n    value is in Octet: buffer.item\n    return value as Octet\n".parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast.qualify_module_defs("sample"), FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn named_membership_proves_narrowing_to_unsigned_domain() {
    let typed = check_parsed(
        "Word is in -128 to 127\nWord has\n    finish(self):\n        return\nOctet is in 0 to 255\nOctet has\n    finish(self):\n        return\nconvert(value Word) Octet: return when value is in Octet then value as Octet else 0\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn checked_cast_can_rebind_a_projected_integer() {
    let typed = check_parsed(
        "Word is in -128 to 127\nWord has\n    finish(self):\n        return\nshrink(value Word) Word:\n    number is in Word: value\n    while number >= 10 or number <= -10 do\n        number :: (number / 10) as Word\n    return number as Word\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn cast_does_not_hide_out_of_range_rebinding() {
    let typed = check_parsed(&format!(
        "{}Digit is in 0 to 9\nbad(value Digit) Digit:\n    number is in Digit: value\n    number :: 255 as Word\n    return number as Digit\n",
        test_fixtures::preludes::WORD_PRELUDE
    ));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-rebind-violates-validity"),
        "{:?}",
        typed.all_flaws()
    );
}
