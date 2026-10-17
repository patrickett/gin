use super::*;
use parser::query::SourceParseExt;

fn format_ast(src: &str) -> String {
    let out = src.parse_source_full();
    AstFormatter::new(src, &Config::default(), &out.ast.span_table).format_file(&out.ast)
}

#[test]
fn inclusive_ranges_round_trip_with_to() {
    let source = "Byte is in 0 to 255\nWindow is -10 to 10\nvalues := 1 to 10\n";
    let formatted = crate::format(source);
    assert!(formatted.contains("Byte is in 0 to 255"));
    assert!(formatted.contains("Window is -10 to 10"));
    assert!(formatted.contains("1 to 10"));
    assert_eq!(crate::format(&formatted), formatted);
    assert!(formatted.parse_source_full().symptoms.is_empty());
}

#[test]
fn unsupported_interpolation_round_trips_with_its_diagnostic() {
    let source = "first := 'John'\nlast := 'Doe'\nfull_name := \"\\(first) \\(last)\"\ncall := \"call(a)\"\nescaped := \"\\\\\\(literal)\"\n";
    let formatted = crate::format(source);
    assert!(formatted.contains("call(a)"));
    assert!(formatted.contains("\\(first) \\(last)"));
    assert_eq!(crate::format(&formatted), formatted);
    assert!(
        formatted
            .parse_source_full()
            .symptoms
            .iter()
            .any(|diagnostic| { diagnostic.code.slug() == "parse-template-strings-not-supported" })
    );
}

#[test]
fn test_basic_declare() {
    let source = "Maybe is Some or None\nResult is Ok or Error\n";
    assert_eq!(
        format_ast(source),
        "Maybe is Some or None\nResult is Ok or Error\n"
    );
}

#[test]
fn test_has_members_use_canonical_no_paren_syntax() {
    let source = "Range(x) has start x, contains(self, value x) Bool\n";
    assert_eq!(
        format_ast(source),
        "Range(x) has start x, contains(self, value x) Bool\n"
    );
}

#[test]
fn composed_interface_preserves_qualified_methods() {
    let source = "TraitA has run(self ref) Int\nTraitB has run(self ref) Int\nCombined has TraitA and TraitB\n    TraitA.run(self ref) Int := return 1\n    TraitB.run(self ref) Int := return 2\n";
    let formatted = format_ast(source);
    assert!(
        formatted.contains("TraitA has run(self ref) Int"),
        "formatted output: {formatted:?}",
    );
    assert_eq!(formatted.matches("TraitA.run").count(), 1);
    assert_eq!(formatted.matches("TraitB.run").count(), 1);
}

#[test]
fn test_simple_bind() {
    let source = "main:\n    print('hello')\n    return\n";
    let r = format_ast(source);
    assert!(r.contains("main:"));
    assert!(r.contains("print('hello')"));
    assert!(r.contains("return"));
}

#[test]
fn formats_named_result_rows() {
    let source = "add(lhs Int, rhs Int) sum Int, overflow Bit: return lhs, rhs\n";
    let formatted = crate::format(source);
    assert!(formatted.contains("add(lhs Int, rhs Int) sum Int, overflow Bit:"));
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn formats_multiple_return_values() {
    let source = "pair() first Int, second Int:\n\treturn 1, 2\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("return 1, 2"),
        "formatted output: {formatted:?}"
    );
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn preserves_return_postconditions() {
    for source in [
        "clear(value mut Int):\n    return thus value = 0\n",
        "positive(value Int):\n    return True thus value > 0\n",
        "less(left Int, right Int):\n    when left < right then\n        return True thus left < right\n    return False thus left >= right\n",
    ] {
        let formatted = crate::format(source);
        let parsed = formatted.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        assert_eq!(
            formatted.matches(" thus ").count(),
            source.matches(" thus ").count()
        );
        assert_eq!(crate::format(&formatted), formatted);
        assert!(matches!(parsed.ast.defs.values().next().unwrap().value,
            ast::BindValue::Body { ref ret, .. } if ret.postcondition.is_some()));
    }
}

#[test]
fn test_empty() {
    let source = "";
    assert_eq!(format_ast(source), "");
}

#[test]
fn test_import_sort_current_module() {
    let source = "use ToString, Copy\n";
    assert_eq!(format_ast(source), "use Copy, ToString\n");
}

#[test]
fn test_import_sort_package() {
    let source = "use http.web, crypto.hash\n";
    assert_eq!(format_ast(source), "use crypto.hash\nuse http.web\n");
}

#[test]
fn test_import_sort_single() {
    let source = "use http\n";
    assert_eq!(format_ast(source), "use http\n");
}

#[test]
fn test_bundle_member_sort() {
    let source = "use core Int, Byte, Area\n";
    assert_eq!(format_ast(source), "use core Area, Byte, Int\n");
}

#[test]
fn test_import_sort_idempotent() {
    let source = "use Copy, ToString\n";
    assert_eq!(format_ast(source), "use Copy, ToString\n");
}

#[test]
fn test_formatter_retains_bind_rebind_and_projection_write_forms() {
    let source = "\
counter Int\n\
counter:: 0\n\
result: counter\n\
pair.0:: counter\n\
pair.0 +: counter + 1\n\
buffer.[0]:: 1\n\
buffer.[0] +: 2\n";
    let formatted = format_ast(source);
    assert!(formatted.contains("counter :: 0"));
    assert!(formatted.contains("result: counter"));
    assert!(formatted.contains("pair.0 :: counter"), "{formatted}");
    assert!(formatted.contains("pair.0 +: counter + 1"));
    assert!(formatted.contains("buffer.[0] :: 1"));
    assert!(formatted.contains("buffer.[0] +: 2"));
}

#[test]
fn compiler_attributes_are_comma_separated_when_they_fit() {
    let source = "#infix('+')\n#inline\nadd: 1\n";
    assert_eq!(format_ast(source), "#infix('+'), #inline\nadd: 1\n");
}

#[test]
fn compiler_attributes_remain_separate_when_they_do_not_fit() {
    let source = "#infix('very-long-operator')\n#inline\nadd: 1\n";
    let out = source.parse_source_full();
    let cfg = Config {
        max_line_width: 30,
        ..Config::default()
    };
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);

    assert_eq!(
        formatter.format_file(&out.ast),
        "#infix('very-long-operator')\n#inline\nadd: 1\n"
    );
}

#[test]
fn formats_intrinsic_integer_definitions_without_duplicating_type_text() {
    let source = "Byte is in 0 to 255\nSignedBigInt is in -9223372036854775808 to 9223372036854775807\n\nsigned_big_int_add_bits(lhs own SignedBigInt, rhs own SignedBigInt) SignedBigInt: return lhs\n#infix_op('+')\n#inline\nsigned_big_int_add(lhs SignedBigInt, rhs SignedBigInt) SignedBigInt: return signed_big_int_add_bits(lhs, rhs)\n";

    let formatted = crate::format(source);
    assert!(formatted.contains("Byte is in 0 to 255"));
    assert!(formatted.contains("SignedBigInt is in -9223372036854775808 to 9223372036854775807"));
    assert!(!formatted.contains("Byte is Byte is"));
    assert!(!formatted.contains("SignedBigInt is SignedBigInt is"));
    let reformatted = crate::format(&formatted);
    assert!(!reformatted.contains("Byte is Byte is"));
    assert!(!reformatted.contains("SignedBigInt is SignedBigInt is"));
}

#[test]
fn separates_top_level_binds_with_one_blank_line() {
    let source = "first: 1\nsecond: 2\nthird: 3\n";
    assert_eq!(crate::format(source), "first: 1\n\nsecond: 2\n\nthird: 3\n");
}

#[test]
fn aligns_types_in_has_properties() {
    let source = "Vec(element, allocator: GlobalAllocator) has\n    storage Allocation(element)\n    length Count(element)\n";
    assert_eq!(
        crate::format(source),
        "Vec(element, allocator: GlobalAllocator) has\n    storage Allocation(element)\n    length  Count(element)\n"
    );
}

#[test]
fn normalizes_parameter_spacing_and_modifier_order() {
    let source = "f(lhs own Int, rhs ref Int, output mut Int) Int: return lhs\n";
    assert_eq!(
        crate::format(source),
        "f(lhs own Int, rhs ref Int, output mut Int) Int: return lhs\n"
    );
}

#[test]
fn normalizes_single_space_after_parameter_commas() {
    let source = "Allocator has reserve(self mut, x, count Count(x)) Allocation(x) or AllocError\n";
    let formatted = crate::format(source);
    assert!(
        !formatted.contains(",  "),
        "formatted output: {formatted:?}"
    );
    assert!(formatted.contains("reserve(self mut, x, count Count(x)) Allocation(x) or AllocError"));
}

#[test]
fn canonicalizes_return_only_constructor_bindings() {
    let source = "Contract has value Int\nmake() Contract:\n    return Contract(value: 1)\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("make Contract: return Contract(value: 1)"),
        "{formatted}"
    );
    assert!(formatted.contains("return"));
    assert_eq!(crate::format(&formatted), formatted);
    assert!(formatted.parse_source_full().symptoms.is_empty());
}

#[test]
fn keeps_contextual_return_types() {
    let source = "value() Int:\n    return 1\n";
    assert_eq!(crate::format(source), "value Int: return 1\n");
}

#[test]
fn terminal_returns_are_indented_inside_the_binding() {
    for result in ["", " value", " value, value"] {
        let source = format!("run():\n    value := 1\n    return{result}\n");
        let formatted = crate::format(&source);
        assert!(
            formatted.contains(&format!("\n    return{result}\n")),
            "{formatted}"
        );
        assert_eq!(crate::format(&formatted), formatted);
        assert!(formatted.parse_source_full().symptoms.is_empty());
    }
}

#[test]
fn return_only_methods_preserve_the_receiver() {
    let source = "Container has value Int\nContainer.build(self) Int:\n    return self.value\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("Container.build(self) Int: return self.value"),
        "{formatted}"
    );
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn preserves_return_only_unsupported_interpolation() {
    let source = r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
Message has protocol Template
Message.new(target Slice(ViewByte), dialect Slice(ViewByte), template Slice(ViewByte)) Message:
    return Message(protocol: "version 1\ntarget \"\(target)\"\ndialect \(dialect)\ntemplate \"\(template)\"")
"#;
    let formatted = crate::format(source);
    assert!(formatted.contains("Message.new("), "{formatted}");
    assert_eq!(crate::format(&formatted), formatted);
    assert!(
        formatted
            .parse_source_full()
            .symptoms
            .iter()
            .any(|diagnostic| { diagnostic.code.slug() == "parse-template-strings-not-supported" })
    );
}

#[test]
fn plain_double_quoted_bytes_preserve_their_value_when_formatted() {
    for value in [r#""it's text""#, r#""line\nnext\0""#, r#""'\\(literal)""#] {
        let source = format!("text: identity({value})\n");
        let formatted = crate::format(&source);
        let parsed = formatted.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{formatted}: {:?}",
            parsed.symptoms
        );
        assert_eq!(crate::format(&formatted), formatted);
    }
}

#[test]
fn multiline_constructor_returns_preserve_generic_annotations() {
    let source = "make() Box(Int):\n    return Box(\n        value: 1\n    )\n";
    let formatted = crate::format(source);
    assert_eq!(formatted, "make Box(Int): return Box(\n    value: 1\n)\n");
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn long_method_parameters_preserve_bare_self() {
    let source = r#"ViewByte is in 0 to 255
ViewLength is in 0 to 18446744073709551615
Slice(element) has ptr @element, len ViewLength
Container.with_parameter(self, parameter Slice(ViewByte), register Slice(ViewByte)) Int:
    return 1
"#;
    let formatted = crate::format(source);
    assert!(
        formatted.contains(
            "(\n    self\n    parameter Slice(ViewByte)\n    register Slice(ViewByte)\n)"
        ),
        "{formatted}"
    );
    assert!(formatted.contains("return"), "{formatted}");
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn return_only_bodies_preserve_immutable_bindings() {
    let source = "value() Int :=\n    return 1\n";
    let formatted = crate::format(source);
    assert_eq!(formatted, "value Int := return 1\n");
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn preserves_encoder_bodies_and_array_representation_attributes() {
    let source = "Encoder has\n    encode(value is in 0 to 255) Bytes(Byte, 1): return [value]\n\nmake() Box:\n    return Box(value: 1)\n\n#fixed_array(element, count)\nBytes(element, count BigInt) has\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("encode(value is in 0 to 255) Bytes(Byte, 1): return [value]"),
        "{formatted}"
    );
    assert!(
        formatted.contains("#fixed_array(element, count)"),
        "{formatted}"
    );
    assert!(formatted.contains("return"), "{formatted}");
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn has_layout_respects_the_configured_width_boundary() {
    let source = "Pair has a Int, longer Int\n";
    for width in [source.trim_end().len(), source.trim_end().len() - 1] {
        let config = Config {
            max_line_width: width,
            ..Config::default()
        };
        let formatted = crate::format_with_config(source, config.clone());
        let expected = if width == source.trim_end().len() {
            source
        } else {
            "Pair has\n    a      Int\n    longer Int\n"
        };
        assert_eq!(formatted, expected);
        assert_eq!(crate::format_with_config(&formatted, config), formatted);
    }
}

#[test]
fn aligns_fields_before_methods_with_default_parameters_and_bodies() {
    let source =
        "Reader has\n    a Int\n    longer Int\n\n    read(self, limit: 10) Int: return limit\n";
    let formatted = crate::format(source);
    assert_eq!(
        formatted,
        "Reader has\n    a      Int\n    longer Int\n\n    read(self, limit: 10) Int: return limit\n"
    );
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn long_field_names_do_not_create_excessive_padding() {
    let source = "Shape has\n    a Int\n    bb Int\n    exceptionally_long_field_name Int\n    c Int\n    ddd Int\n";
    let formatted = crate::format(source);
    assert_eq!(
        formatted,
        "Shape has\n    a  Int\n    bb Int\n    exceptionally_long_field_name Int\n    c   Int\n    ddd Int\n"
    );
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn type_alignment_padding_is_configurable() {
    let source = "Shape has\n    a Int\n    longer Int\n";
    let config = Config {
        max_type_alignment_padding: 0,
        ..Config::default()
    };
    assert_eq!(crate::format_with_config(source, config), source);
}

#[test]
fn alignment_does_not_push_a_type_over_the_width_limit() {
    let source = "Shape has\n    a FairlyLongType\n    longer Int\n";
    let config = Config {
        max_line_width: 20,
        ..Config::default()
    };
    let formatted = crate::format_with_config(source, config.clone());
    assert_eq!(formatted, source);
    assert_eq!(crate::format_with_config(&formatted, config), formatted);
}

#[test]
fn generic_has_headers_and_method_signatures_wrap_without_losing_fields() {
    let source = "Record(element, count BigInt) has value element, size BigInt, read(self, limit: 10) element\n";
    let config = Config {
        max_line_width: 60,
        ..Config::default()
    };
    let formatted = crate::format_with_config(source, config.clone());
    assert!(
        formatted.contains(
            " has\n    value element\n    size  BigInt\n    read(self, limit: 10) element"
        ),
        "{formatted}"
    );
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format_with_config(&formatted, config), formatted);
}

#[test]
fn multiline_generic_headers_use_block_members_even_when_members_fit_inline() {
    let source = "Region(contract_type, item_type, count BigInt) has contract contract_type, items item_type\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains(") has\n    contract contract_type\n    items    item_type"),
        "{formatted}"
    );
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn local_bundle_imports_allow_attribute_and_return_normalization() {
    let source = "use '../arithmetic' Word, add\n\n#infix('+')\n#inline\ncombine(lhs Word, rhs Word) Word:\n    return add(lhs, rhs) as Word\n";
    let attributes = "#infix('+'), #inline";
    for width in [80, attributes.len(), attributes.len() - 1] {
        let config = Config {
            max_line_width: width,
            ..Config::default()
        };
        let formatted = crate::format_with_config(source, config.clone());
        assert!(
            formatted.starts_with(if width >= "use '../arithmetic' add, Word".len() {
                "use '../arithmetic' add, Word\n"
            } else {
                "use '../arithmetic'\n    add, Word\n"
            }),
            "{formatted}"
        );
        let expected_attributes = if width >= attributes.len() {
            attributes
        } else {
            "#infix('+')\n#inline"
        };
        assert!(formatted.contains(expected_attributes), "{formatted}");
        assert!(
            formatted.contains(if width == 80 {
                "Word: return add(lhs, rhs) as Word"
            } else {
                "Word: return add(\n    lhs\n    rhs\n) as Word"
            }),
            "{formatted}"
        );
        assert!(formatted.contains("return "), "{formatted}");
        assert!(formatted.parse_source_full().symptoms.is_empty());
        assert_eq!(crate::format_with_config(&formatted, config), formatted);
    }
}

#[test]
fn local_bundle_paths_and_member_aliases_survive_sorting() {
    let source = "use '../zeta' Zed as Z, Alpha\nuse '../alpha' Beta as B\n";
    let formatted = crate::format(source);
    assert_eq!(
        formatted,
        "use '../zeta' Alpha, Zed as Z\nuse '../alpha' Beta as B\n"
    );
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn import_members_wrap_without_parentheses_and_round_trip() {
    let source = "use '../math' subtract as difference, add\n\nvalue := 1\n";
    let cfg = Config {
        max_line_width: 30,
        ..Config::default()
    };
    let out = source.parse_source_full();
    assert!(out.symptoms.is_empty(), "{:?}", out.symptoms);
    let formatted = AstFormatter::new(source, &cfg, &out.ast.span_table).format_file(&out.ast);
    assert!(
        formatted.starts_with("use '../math'\n    add,\n    subtract as difference\n"),
        "{formatted}"
    );
    let reparsed = formatted.parse_source_full();
    assert!(reparsed.symptoms.is_empty(), "{:?}", reparsed.symptoms);
    let again =
        AstFormatter::new(&formatted, &cfg, &reparsed.ast.span_table).format_file(&reparsed.ast);
    assert_eq!(formatted, again);
}

#[test]
fn has_method_result_alternatives_round_trip() {
    let source = "Compare has\n    compare(left, right) Lower thus left < right or Equal thus left = right or Higher thus left > right\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains(
            "Lower thus left < right or Equal thus left = right or Higher thus left > right"
        ),
        "{formatted}"
    );
    assert_eq!(crate::format(&formatted), formatted);
    assert!(formatted.parse_source_full().symptoms.is_empty());
}

#[test]
fn self_and_nested_result_alternatives_round_trip() {
    let source = "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\n\nWord has\n    copy(value ref Self) Self:\n        bits is in 0 to 255: value\n        return bits as Self\n";
    let formatted = crate::format(source);
    assert_eq!(crate::format(&formatted), formatted);
    assert!(
        formatted.contains("Outcome is Word or Failure(Empty or Full)"),
        "{formatted}"
    );
    assert!(
        formatted.contains("copy(value ref Self) Self"),
        "{formatted}"
    );
}

#[test]
fn sequence_forms_round_trip_without_adding_result_brackets() {
    for source in [
        "numbers: 10, 20\n",
        "point: x: 10, y: 20\n",
        "point: [x: 10, 20]\n",
        "make() first Int, Int: return 1, 2\n",
        "make() Int, second Int: return 1, 2\n",
        "make() values [Int; 2], count Int: return [1, 2], 2\n",
    ] {
        let formatted = crate::format(source);
        assert_eq!(crate::format(&formatted), formatted, "{source}");
        let parsed = formatted.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .all(|d| d.category != diagnostic::Category::Flaw),
            "{formatted}: {:?}",
            parsed.symptoms
        );
    }
}

#[test]
fn integer_domain_annotations_round_trip() {
    let source = "Word is in -128 to 127\nidentity(value is in Word) is in Word: return value\nHolder has\n    raw(self):\n        value is in Self: self\n        return value\n";
    let formatted = crate::format(source);
    assert!(formatted.contains("value is in Word"), "{formatted}");
    assert!(
        formatted.contains("is in Word: return value"),
        "{formatted}"
    );
    assert!(formatted.contains("value is in Self: self"), "{formatted}");
    assert!(
        formatted.parse_source_full().symptoms.is_empty(),
        "{formatted}"
    );
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn generic_trait_arguments_round_trip() {
    let source = "Div(result) has\n    divide(self, rhs Self) result\nExample has Div(Outcome)\n    divide(self, rhs Self) Outcome: return rhs\n";
    let formatted = crate::format(source);
    assert!(formatted.contains("Div(Outcome)"), "{formatted}");
    assert_eq!(crate::format(&formatted), formatted);
    assert!(formatted.parse_source_full().symptoms.is_empty());
}

#[test]
fn trait_chains_wrap_at_the_width_boundary_and_align_with_has() {
    let header = "Number has Add and Mul(Result) and Copy";
    for body in [
        "",
        "\n    apply(self, rhs Self): return rhs\n    other(self): return self",
    ] {
        let source = format!("{header}{body}\n");
        for (width, expected_header) in [
            (header.len(), header),
            (
                header.len() - 1,
                "Number has Add and Mul(Result)\n       and Copy",
            ),
            (
                25,
                "Number has Add\n       and Mul(Result)\n       and Copy",
            ),
        ] {
            let config = Config {
                max_line_width: width,
                ..Config::default()
            };
            let formatted = crate::format_with_config(&source, config.clone());
            assert_eq!(formatted, format!("{expected_header}{body}\n"));
            assert!(
                formatted.parse_source_full().symptoms.is_empty(),
                "{formatted}"
            );
            assert_eq!(crate::format_with_config(&formatted, config), formatted);
        }
    }
}

#[test]
fn wrapped_generic_trait_headers_preserve_members() {
    let source = "Container(element) has Reader(element) and Writer(element) and Copy\n    read(self) element\n";
    let config = Config {
        max_line_width: 45,
        ..Config::default()
    };
    let formatted = crate::format_with_config(source, config.clone());
    assert_eq!(
        formatted,
        "Container(element) has Reader(element)\n                   and Writer(element)\n                   and Copy\n    read(self) element\n"
    );
    assert!(
        formatted.parse_source_full().symptoms.is_empty(),
        "{formatted}"
    );
    assert_eq!(crate::format_with_config(&formatted, config), formatted);
}

#[test]
fn import_members_fill_continuation_lines_at_configured_width() {
    let source = "use sample.ops Gamma as G, Delta, Beta, Alpha\n";
    for (width, expected) in [
        (48, "use sample.ops Alpha, Beta, Delta, Gamma as G\n"),
        (45, "use sample.ops Alpha, Beta, Delta, Gamma as G\n"),
        (44, "use sample.ops\n    Alpha, Beta, Delta, Gamma as G\n"),
        (
            23,
            "use sample.ops\n    Alpha, Beta, Delta,\n    Gamma as G\n",
        ),
        (
            22,
            "use sample.ops\n    Alpha, Beta,\n    Delta, Gamma as G\n",
        ),
    ] {
        let cfg = Config {
            max_line_width: width,
            ..Config::default()
        };
        let formatted = crate::format_with_config(source, cfg.clone());
        assert_eq!(formatted, expected, "width {width}");
        assert!(formatted.lines().all(|line| line.len() <= width));
        let parsed = formatted.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        assert_eq!(crate::format_with_config(&formatted, cfg), formatted);
    }
}

#[test]
fn numeric_predicates_round_trip() {
    let source =
        "choose(value is in 0 to 255): return when value < 16 and value in 0 to 15 then 1 else 0\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("value < 16 and value in 0 to 15"),
        "{formatted}"
    );
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format(&formatted), formatted);
}
