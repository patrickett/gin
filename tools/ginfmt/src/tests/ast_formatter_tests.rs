use super::*;
use parser::query::SourceParseExt;

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
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "Maybe is Some or None\nResult is Ok or Error\n");
}

#[test]
fn test_has_members_use_canonical_no_paren_syntax() {
    let source = "Range(x) has start x, contains(self, value x) Bool\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "Range(x) has start x, contains(self, value x) Bool\n");
}

#[test]
fn composed_interface_preserves_qualified_methods() {
    let source = "TraitA has run(self ref) Int\nTraitB has run(self ref) Int\nCombined has TraitA and TraitB\n    TraitA.run(self ref) Int := 1\n    TraitB.run(self ref) Int := 2\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let formatted = formatter.format_file(&out.ast);
    assert!(
        formatted.contains("TraitA has run(self ref) Int"),
        "formatted output: {formatted:?}",
    );
    assert_eq!(formatted.matches("TraitA.run").count(), 1);
    assert_eq!(formatted.matches("TraitB.run").count(), 1);
}

#[test]
fn test_simple_bind() {
    let source = "main:\n    print('hello')\nreturn\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert!(r.contains("main:"));
    assert!(r.contains("print('hello')"));
    assert!(r.contains("return"));
}

#[test]
fn formats_named_result_rows() {
    let source = "add(lhs Int, rhs Int) sum Int, overflow Bit: lhs, rhs\n";
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
fn test_empty() {
    let source = "";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "");
}

#[test]
fn test_import_sort_current_module() {
    let source = "use ToString, Copy\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use Copy, ToString\n");
}

#[test]
fn test_import_sort_package() {
    let source = "use http.web, crypto.hash\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use crypto.hash\nuse http.web\n");
}

#[test]
fn test_import_sort_single() {
    let source = "use http\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use http\n");
}

#[test]
fn test_bundle_member_sort() {
    let source = "use core Int, Byte, Area\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use core Area, Byte, Int\n");
}

#[test]
fn test_import_sort_idempotent() {
    let source = "use Copy, ToString\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut f = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let r = f.format_file(&out.ast);
    assert_eq!(r, "use Copy, ToString\n");
}

#[test]
fn test_formatter_retains_bind_rebind_and_projection_write_forms() {
    let source = "\
counter Int\n\
counter:: 0\n\
result: counter\n\
pair.0:: counter\n\
pair.0 +: counter + 1\n\
buffer.(0):: 1\n\
buffer.(0) +: 2\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);
    let formatted = formatter.format_file(&out.ast);
    assert!(formatted.contains("counter:: 0"));
    assert!(formatted.contains("result: counter"));
    assert!(formatted.contains("pair.0:: counter"), "{formatted}");
    assert!(formatted.contains("pair.0 +: counter + 1"));
    assert!(formatted.contains("buffer.(0):: 1"));
    assert!(formatted.contains("buffer.(0) +: 2"));
}

#[test]
fn compiler_attributes_are_comma_separated_when_they_fit() {
    let source = "#infix('+')\n#inline\nadd: 1\n";
    let out = source.parse_source_full();
    let cfg = Config::default();
    let mut formatter = AstFormatter::new(source, &cfg, &out.ast.span_table);

    assert_eq!(
        formatter.format_file(&out.ast),
        "#infix('+'), #inline\nadd: 1\n"
    );
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
    let source = "Byte is in 0 to 255\nSignedBigInt is in -9223372036854775808 to 9223372036854775807\n\nsigned_big_int_add_bits(lhs own SignedBigInt, rhs own SignedBigInt) SignedBigInt: lhs\n#infix_op('+')\n#inline\nsigned_big_int_add(lhs SignedBigInt, rhs SignedBigInt) SignedBigInt: signed_big_int_add_bits(lhs, rhs)\n";

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
    let source = "Vec(element Type, allocator: GlobalAllocator) has\n    storage Allocation(element)\n    length Count(element)\n";
    assert_eq!(
        crate::format(source),
        "Vec(element Type, allocator: GlobalAllocator) has\n    storage Allocation(element)\n    length  Count(element)\n"
    );
}

#[test]
fn normalizes_parameter_spacing_and_modifier_order() {
    let source = "f(lhs own Int, rhs ref Int, output mut Int) Int: lhs\n";
    assert_eq!(
        crate::format(source),
        "f(lhs own Int, rhs ref Int, output mut Int) Int: lhs\n"
    );
}

#[test]
fn normalizes_single_space_after_parameter_commas() {
    let source = "Allocator has reserve(self mut, element Type, count Count(element)) AllocAttempt(element)\n";
    let formatted = crate::format(source);
    assert!(
        !formatted.contains(",  "),
        "formatted output: {formatted:?}"
    );
    assert!(
        formatted.contains(
            "reserve(self mut, element Type, count Count(element)) AllocAttempt(element)"
        )
    );
}

#[test]
fn canonicalizes_return_only_constructor_bindings() {
    let source = "Contract has value Int\nmake() Contract:\n    return Contract(value: 1)\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("make: Contract(value: 1)"),
        "{formatted}"
    );
    assert!(!formatted.contains("return"));
    assert_eq!(crate::format(&formatted), formatted);
    assert!(formatted.parse_source_full().symptoms.is_empty());
}

#[test]
fn keeps_contextual_return_types() {
    let source = "value() Int:\n    return 1\n";
    assert_eq!(crate::format(source), "value Int: 1\n");
}

#[test]
fn terminal_returns_align_with_the_binding() {
    for result in ["", " value", " value, value"] {
        let source = format!("run():\n    value := 1\n    return{result}\n");
        let formatted = crate::format(&source);
        assert!(
            formatted.contains(&format!("\nreturn{result}\n")),
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
        formatted.contains("Container.build(self) Int: self.value"),
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
    assert_eq!(formatted, "make Box(Int): Box(value: 1)\n");
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
        formatted.contains(r#"(self, parameter Slice(ViewByte), register Slice(ViewByte))"#),
        "{formatted}"
    );
    assert!(!formatted.contains("return"), "{formatted}");
    assert!(formatted.parse_source_full().symptoms.is_empty());
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn return_only_bodies_preserve_immutable_bindings() {
    let source = "value() Int :=\n    return 1\n";
    let formatted = crate::format(source);
    assert_eq!(formatted, "value Int := 1\n");
    assert_eq!(crate::format(&formatted), formatted);
}

#[test]
fn preserves_encoder_bodies_and_array_representation_attributes() {
    let source = "Encoder has\n    encode(value is in 0 to 255) Bytes(Byte, 1): [value]\n\nmake() Box:\n    return Box(value: 1)\n\n#fixed_array(element, count)\nBytes(element Type, count BigInt) has\n";
    let formatted = crate::format(source);
    assert!(
        formatted.contains("encode(value is in 0 to 255) Bytes(Byte, 1): [value]"),
        "{formatted}"
    );
    assert!(
        formatted.contains("#fixed_array(element, count)"),
        "{formatted}"
    );
    assert!(!formatted.contains("return"), "{formatted}");
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
    let source = "Reader has\n    a Int\n    longer Int\n\n    read(self, limit: 10) Int: limit\n";
    let formatted = crate::format(source);
    assert_eq!(
        formatted,
        "Reader has\n    a      Int\n    longer Int\n\n    read(self, limit: 10) Int: limit\n"
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
    let source = "Record(element Type, count BigInt) has value element, size BigInt, read(self, limit: 10) element\n";
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
    let source = "Region(contract_type Type, item_type Type, count BigInt) has contract contract_type, items item_type\n";
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
                "use '../arithmetic'\n    add\n    Word\n"
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
            formatted.contains("Word: add(lhs, rhs) as Word"),
            "{formatted}"
        );
        assert!(!formatted.contains("return "), "{formatted}");
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
        formatted.starts_with("use '../math'\n    add\n    subtract as difference\n"),
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
    let source = "Word is in 0 to 255\nOutcome is Word or Failure(Empty or Full)\n\nWord has\n    copy(value ref Self) Self:\n        bits is in 0 to 255: value\n    return bits as Self\n";
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
