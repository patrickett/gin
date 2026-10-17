use diagnostic::Category;
use parser::query::SourceParseExt;

fn assert_parses(case: &str, source: &str) {
    let output = source.parse_source_full();
    let flaws = output
        .symptoms
        .iter()
        .filter(|diagnostic| diagnostic.category == Category::Flaw)
        .map(|diagnostic| format!("{}: {}", diagnostic.code.slug(), diagnostic.message))
        .collect::<Vec<_>>();
    assert!(
        flaws.is_empty(),
        "case `{case}` failed:\n{}\n{source}",
        flaws.join("; ")
    );
}

const CASES: &[(&str, &str)] = &[
    (
        "inferred_integer_binding",
        r#"value: 2
"#,
    ),
    (
        "typed_integer_binding",
        r#"value Int: 2
"#,
    ),
    (
        "combined_integer_refinements",
        r#"value Int and > 0: 2
index is in 0 to 255: 0
index is >= 0 and <= 255: 0
non_five is >= 0 and not = 5 and < 100: input()
"#,
    ),
    (
        "range_excluding_value",
        r#"value is in 0 to 10 and not = 5: input()
"#,
    ),
    (
        "positive_integer_refinement",
        r#"value is > 0: 2
"#,
    ),
    (
        "explicit_integer_bit_width",
        r#"#bits(16)
MyInt is in 0 to 256
"#,
    ),
    (
        "inferred_integer_arithmetic",
        r#"a: 250
b: a + 10
"#,
    ),
    (
        "contextual_integer_arithmetic",
        r#"result TinyInt: 250 + 10
"#,
    ),
    (
        "modular_integer_arithmetic",
        r#"a ModularByte: 250
b ModularByte: 10
result: a + b
"#,
    ),
    (
        "dependent_integer_arithmetic",
        r#"x is in 0 to 10: input()
y: 10 - x
z: x + y
"#,
    ),
    (
        "typed_conversion_binding",
        r#"converted Int: value
"#,
    ),
    (
        "explicit_integer_conversion",
        r#"converted := value as Int
"#,
    ),
    (
        "comparison_operator_declaration",
        r#"#infix('<')
less_than( self ref Int, rhs ref Int) True or False: return True
"#,
    ),
    (
        "comparison_result_condition",
        r#"check( left ref Int, right ref Int):
    result := left < right
    when result is True then
        observe(left)
        return
    return 0
"#,
    ),
    (
        "forwarded_return_value",
        r#"forward(value):
    return value
"#,
    ),
    (
        "comparison_function_result",
        r#"is_less( left ref Int, right ref Int) True or False:
    when left < right then
        True
        return
    return False
"#,
    ),
    (
        "comparison_proof_contract",
        r#"#infix('<')
less_than( self ref Int, rhs ref Int) True thus self < rhs or False thus self >= rhs: return True
"#,
    ),
    (
        "signed_integer_result_proofs",
        r#"SignedByte is in -128 to 127

sign( value ref SignedByte) Negative thus value < 0 or Nonnegative thus value >= 0:
    when value is in -128 to -1 then
        Negative
        return
    return Nonnegative
"#,
    ),
    (
        "equality_pattern_condition",
        r#"check(value Int):
    when value = 10 is Equal then
        observe(value)
        return
    return 0
"#,
    ),
    (
        "conjoined_pattern_conditions",
        r#"check( left ref Int, right ref Int, value Int):
    when left < right and value is in 0 to 10 then
        observe(value)
        return
    return 0
"#,
    ),
    (
        "fresh_binding_shadowing",
        r#"value: 2
value: transform(value)
"#,
    ),
    (
        "place_rebinding",
        r#"update():
    value:: next
    point.x:: 10
    items.[index]:: replacement
    return 0
"#,
    ),
    (
        "compound_rebinding",
        r#"counter +: 1
"#,
    ),
    (
        "linear_handle_shadowing",
        r#"handle: open()
handle: reopen(handle)
"#,
    ),
    (
        "loop_rebinding",
        r#"counter: 0

while running is True do
    counter +: 1
"#,
    ),
    (
        "refined_loop_counter",
        r#"counter is in 0 to 100: 0

while counter < 100 do
    counter +: 1
"#,
    ),
    (
        "target_dependent_integer_types",
        r#"index_bits := #index_bits(@())
index_sign := 1 << (index_bits - 1)
max_count := index_sign - 1
min_offset := 0 - max_count
max_alignment := 1 << (index_bits - 2)

#phantom(unit)
#bits(index_bits)
Count(unit) is in 0 to max_count

#phantom(unit)
#bits(index_bits)
Offset(unit) is in min_offset to max_count

#bits(index_bits)
Alignment is in 1 to max_alignment and max_alignment % self = 0

ByteCount is Count(Byte)
ByteOffset is Offset(Byte)
"#,
    ),
    (
        "validated_alignment_and_layout",
        r#"is_alignment( bytes ref is in 0 to max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:
    when bytes is in 1 to max_alignment then
        candidate is in 1 to max_alignment and max_alignment % candidate = 0: 1
        while candidate < bytes do
            candidate *: 2
        when candidate = bytes then
            True
            return
        False
        return
    return False

Alignment has
    new(bytes is in 1 to max_alignment and max_alignment % bytes = 0) Self:
        return bytes as Alignment

    try_new(bytes is in 0 to max_count):
        when is_alignment( bytes) is True then
            Valid(bytes as Alignment)
            return
        return Invalid

Layout(element) has
    count Count(element) and * #size(element) <= max_count
    alignment Alignment and >= #alignment(element)

    new(count Count(element) and * #size(element) <= max_count) Self:
        return Self(
            count: count
            alignment: Alignment.new(#alignment(element))
        )

    new(
        count Count(element) and * #size(element) <= max_count
        alignment Alignment and >= #alignment(element)
    ) Self:
        return Self(count: count, alignment: alignment)

    try_new(count is in 0 to max_count):
        return Self.try_new(count: count, alignment: #alignment(element))

    try_new(count is in 0 to max_count, alignment is in 0 to max_count):
        when is_alignment( alignment) is True and alignment >= #alignment(element) and count * #size(element) <= max_count then
            Valid(Self(
                count: count as Count(element)
                alignment: alignment as Alignment
            ))
            return
        return Invalid
"#,
    ),
    (
        "tab_indented_equality_condition",
        r#"check(value Int):
	when value = 10 is Equal then
		observe(value)
		return
	return 0
"#,
    ),
    (
        "forwarded_comparison_result",
        r#"compare( left ref Int, right ref Int):
    return left < right
"#,
    ),
    (
        "tab_indented_comparison_result",
        r#"is_less( left ref Int, right ref Int) True or False:
	when left < right then
		True
		return
	return False
"#,
    ),
    (
        "tab_indented_conjoined_conditions",
        r#"check( left ref Int, right ref Int, value Int):
	when left < right and value is in 0 to 10 then
		observe(value)
		return
	return 0
"#,
    ),
    (
        "linear_handle_consumption",
        r#"main:
	handle: open()
	close(handle)
	return 0
"#,
    ),
    (
        "linear_handle_replacement",
        r#"handle: reopen(handle)
"#,
    ),
    (
        "shadowing_transformed_value",
        r#"value: transform(value)
"#,
    ),
    (
        "tab_indented_place_rebinding",
        r#"update():
	counter:: next
	point.x:: 10
	pair.[0]:: value
	items.[index]:: replacement
	return 0
"#,
    ),
    (
        "linear_handle_rebinding",
        r#"handle:: reopen(handle)
"#,
    ),
];

#[test]
fn syntax_regression_cases() {
    for &(name, source) in CASES {
        assert_parses(name, source);
    }
}
