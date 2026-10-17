mod support;

use internment::Intern;
use parser::query::SourceParseExt;
use support::check_parsed;
use typecheck::{DefId, FileId, transform::transform_file};

#[test]
fn operator_trait_substitutes_result_argument() {
    let typed = check_parsed(
        r#"
Div(result) has
    #infix('/'), #inline
    divide(self, rhs Self) result
Word is in 0 to 9
Outcome is Word or Flaw(Zero or Overflow)
Outcome.finish(self):
    return
Word has Div(Outcome)
    divide(self is in Self, rhs is in Self): return when rhs = 0 then Flaw(Zero) else (self / rhs) as Self
quotient(left Word, right Word) Outcome: return left / right
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let method = &typed.defs[&DefId::named(Intern::from_ref("Word.divide"))];
    assert_eq!(
        method.return_type,
        typed.defs[&DefId::named(Intern::from_ref("quotient"))].return_type
    );
    assert!(method.attributes.inline_always);
}

#[test]
fn inherited_integer_copy_default_is_specialized() {
    let typed = check_parsed(
        r#"
Copy has
    copy(self ref) Self: return (bits is in Self: self) as Self
Copy.finish(self):
    return
Word is in -128 to 127
Word.finish(self):
    return
Word has Copy
copy_word(value ref Word) Word: return Word.copy(value)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(
        typed
            .defs
            .contains_key(&DefId::named(Intern::from_ref("Word.copy")))
    );
}

#[test]
fn explicit_copy_implementation_overrides_default() {
    let typed = check_parsed(
        r#"
Copy has
    copy(self ref) Self:
        bits is in Self: self
        return bits as Self
Copy.finish(self):
    return
Word is in 0 to 9
Word has Copy
    finish(self):
        return
    copy(value ref Self) Self: return 0 as Self
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn trait_arguments_must_have_the_declared_arity() {
    for provision in ["Convert", "Convert(Word, Word)"] {
        let typed = check_parsed(&format!(
            "Convert(result) has\n    convert(self) result\nWord is in 0 to 9\nWord has {provision}\n    convert(self) Self: return self\n"
        ));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, d)| d.code.slug() == "type-trait-argument-count"),
            "{:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn integer_copy_default_rejects_records() {
    let source = r#"
Copy has
    copy(self ref) Self: return (bits is in Self: self) as Self
Copy.finish(self):
    return
Word is in 0 to 9
Word.finish(self):
    return
Record has
    field Word
Record has Copy
    finish(self):
        return
"#;
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
    assert_eq!(flaw.span.start(), source.find("in Self").unwrap() + 3);
    assert_eq!(flaw.span.end(), flaw.span.start() + "Self".len());
}

#[test]
fn record_copy_override_does_not_require_an_integer_domain() {
    let typed = check_parsed(
        r#"
Copy has
    copy(self ref) Self: return (bits is in Self: self) as Self
Copy.finish(self):
    return
Word is in 0 to 9
Word.finish(self):
    return
Record has field Word
Record has Copy
    finish(self):
        return
    copy(self ref) Self: return Self(field: 0)
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn composed_traits_forward_result_arguments() {
    let typed = check_parsed(
        r#"
Convert(result) has
    convert(self) result
Wrapped(result) has Convert(result)
Word is in 0 to 9
Wide is in 0 to 99
Wide.finish(self):
    return
Word has Wrapped(Wide)
    convert(self): return 1 as Wide
"#,
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn conflicting_default_bodies_require_an_override() {
    let typed = check_parsed(
        r#"
First has
    copy(self ref) Self:
        bits is in Self: self
        return bits as Self
First.finish(self):
    return
Second has
    copy(self ref) Self:
        bits is in Self: self
        return bits as Self
Word is in 0 to 9
Word has First and Second
    finish(self):
        return
Second.finish(self):
    return
"#,
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, d)| d.code.slug() == "type-ambiguous-trait-default"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn imported_traits_supply_generic_contracts_and_defaults() {
    let traits = r#"
Convert(result) has
    convert(self) result
Copy has
    copy(self ref) Self:
        bits is in Self: self
        return bits as Self
Copy.finish(self):
    return
"#
    .parse_source_full();
    assert!(traits.symptoms.is_empty(), "{:?}", traits.symptoms);
    let trait_types = transform_file(traits.ast.clone(), FileId(0));
    let mut ctx = typecheck::transform::TransformCtx::from_typed_asts(&[&trait_types]);
    ctx.compile_time_eval_ast = std::sync::Arc::new(traits.ast);
    let source = "use operations Convert, Copy\nWord is in 0 to 9\nWord has Convert(Word) and Copy\n    convert(self): return self\nuse_copy(value ref Word) Word: return Word.copy(value)\n".parse_source_full();
    assert!(source.symptoms.is_empty(), "{:?}", source.symptoms);
    let typed = typecheck::transform::transform(&source.ast, FileId(1), &ctx);
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn generic_numeric_defaults_resolve_after_destructuring_a_concrete_record() {
    let typed = check_parsed(&format!(
        "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\n    to_num(self) is in Self: return (number is in Self: self)\n{}#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has Numeric\n    finish(self):\n        return\nBlock(element) has\n    capacity Count(element)\n    finish(self):\n        return\nreclaim(block Block(Byte)) is in Count(Byte):\n    Block(capacity) := block\n    return capacity.to_num\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}
