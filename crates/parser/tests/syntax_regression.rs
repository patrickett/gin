use diagnostic::Category;
use parser::query::SourceParseExt;

fn assert_parses(source: &str) {
    let output = source.parse_source_full();
    let flaws = output
        .symptoms
        .iter()
        .filter(|diagnostic| diagnostic.category == Category::Flaw)
        .map(|diagnostic| format!("{}: {}", diagnostic.code.slug(), diagnostic.message))
        .collect::<Vec<_>>();
    assert!(flaws.is_empty(), "{}\n{source}", flaws.join("; "));
}

#[test]
fn inferred_integer_binding() {
    assert_parses(
        r#"value: 2
"#,
    );
}

#[test]
fn typed_integer_binding() {
    assert_parses(
        r#"value Int: 2
"#,
    );
}

#[test]
fn combined_integer_refinements() {
    assert_parses(
        r#"value Int and > 0: 2
index is in 0 to 255: 0
index is >= 0 and <= 255: 0
non_five is >= 0 and not = 5 and < 100: input()
"#,
    );
}

#[test]
fn range_excluding_value() {
    assert_parses(
        r#"value is in 0 to 10 and not = 5: input()
"#,
    );
}

#[test]
fn positive_integer_refinement() {
    assert_parses(
        r#"value is > 0: 2
"#,
    );
}

#[test]
fn explicit_integer_bit_width() {
    assert_parses(
        r#"#bits(16)
MyInt is in 0 to 256
"#,
    );
}

#[test]
fn inferred_integer_arithmetic() {
    assert_parses(
        r#"a: 250
b: a + 10
"#,
    );
}

#[test]
fn contextual_integer_arithmetic() {
    assert_parses(
        r#"result TinyInt: 250 + 10
"#,
    );
}

#[test]
fn modular_integer_arithmetic() {
    assert_parses(
        r#"a ModularByte: 250
b ModularByte: 10
result: a + b
"#,
    );
}

#[test]
fn dependent_integer_arithmetic() {
    assert_parses(
        r#"x is in 0 to 10: input()
y: 10 - x
z: x + y
"#,
    );
}

#[test]
fn typed_conversion_binding() {
    assert_parses(
        r#"converted Int: value
"#,
    );
}

#[test]
fn explicit_integer_conversion() {
    assert_parses(
        r#"converted := value as Int
"#,
    );
}

#[test]
fn comparison_operator_declaration() {
    assert_parses(
        r#"#infix('<')
less_than( self ref Int, rhs ref Int) True or False: True
"#,
    );
}

#[test]
fn comparison_result_condition() {
    assert_parses(
        r#"check( left ref Int, right ref Int):
    result := left < right
    if result is True
        observe(left)
    return
return 0
"#,
    );
}

#[test]
fn forwarded_return_value() {
    assert_parses(
        r#"forward(value):
return value
"#,
    );
}

#[test]
fn comparison_function_result() {
    assert_parses(
        r#"is_less( left ref Int, right ref Int) True or False:
    if left < right is True
        True
    return
return False
"#,
    );
}

#[test]
fn comparison_proof_contract() {
    assert_parses(
        r#"#infix('<')
less_than( self ref Int, rhs ref Int) True thus self < rhs or False thus self >= rhs: True
"#,
    );
}

#[test]
fn signed_integer_result_proofs() {
    assert_parses(
        r#"SignedByte is in -128 to 127

sign( value ref SignedByte) Negative thus value < 0 or Nonnegative thus value >= 0:
    if value is in -128 to -1
        Negative
    return
return Nonnegative
"#,
    );
}

#[test]
fn equality_pattern_condition() {
    assert_parses(
        r#"check(value Int):
    if value = 10 is Equal
        observe(value)
    return
return 0
"#,
    );
}

#[test]
fn conjoined_pattern_conditions() {
    assert_parses(
        r#"check( left ref Int, right ref Int, value Int):
    if left < right is True and value is in 0 to 10
        observe(value)
    return
return 0
"#,
    );
}

#[test]
fn fresh_binding_shadowing() {
    assert_parses(
        r#"value: 2
value: transform(value)
"#,
    );
}

#[test]
fn place_rebinding() {
    assert_parses(
        r#"update():
    value:: next
    point.x:: 10
    items.(index):: replacement
return 0
"#,
    );
}

#[test]
fn compound_rebinding() {
    assert_parses(
        r#"counter +: 1
"#,
    );
}

#[test]
fn linear_handle_shadowing() {
    assert_parses(
        r#"handle: open()
handle: reopen(handle)
"#,
    );
}

#[test]
fn loop_rebinding() {
    assert_parses(
        r#"counter: 0

while running is True
    counter +: 1
loop
"#,
    );
}

#[test]
fn refined_loop_counter() {
    assert_parses(
        r#"counter is in 0 to 100: 0

while counter < 100 is True
    counter +: 1
loop
"#,
    );
}

#[test]
fn target_dependent_integer_types() {
    assert_parses(
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
    );
}

#[test]
fn validated_alignment_and_layout() {
    assert_parses(
        r#"is_alignment( bytes ref is in 0 to max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:
    if bytes is in 1 to max_alignment
        candidate is in 1 to max_alignment and max_alignment % candidate = 0: 1
        while candidate < bytes is True
            candidate *: 2
        loop
        if candidate = bytes is True
            True
        return
        False
    return
return False

Alignment has
    new(bytes is in 1 to max_alignment and max_alignment % bytes = 0) Self:
    return bytes as Alignment

    try_new(bytes is in 0 to max_count):
        if is_alignment( bytes) is True
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
        if is_alignment( alignment) is True and alignment >= #alignment(element) is True and count * #size(element) <= max_count is True
            Valid(Self(
                count: count as Count(element)
                alignment: alignment as Alignment
            ))
        return
    return Invalid
"#,
    );
}

#[test]
fn comparison_conversion() {
    assert_parses(
        r#"converted := value as Int
"#,
    );
}

#[test]
fn tab_indented_equality_condition() {
    assert_parses(
        r#"check(value Int):
	if value = 10 is Equal
		observe(value)
	return
return 0
"#,
    );
}

#[test]
fn forwarded_comparison_result() {
    assert_parses(
        r#"compare( left ref Int, right ref Int):
return left < right
"#,
    );
}

#[test]
fn tab_indented_comparison_result() {
    assert_parses(
        r#"is_less( left ref Int, right ref Int) True or False:
	if left < right is True
		True
	return
return False
"#,
    );
}

#[test]
fn comparison_result_evidence() {
    assert_parses(
        r#"#infix('<')
less_than( self ref Int, rhs ref Int) True thus self < rhs or False thus self >= rhs: True
"#,
    );
}

#[test]
fn tab_indented_conjoined_conditions() {
    assert_parses(
        r#"check( left ref Int, right ref Int, value Int):
	if left < right is True and value is in 0 to 10
		observe(value)
	return
return 0
"#,
    );
}

#[test]
fn linear_handle_consumption() {
    assert_parses(
        r#"main:
	handle: open()
	close(handle)
return 0
"#,
    );
}

#[test]
fn linear_handle_replacement() {
    assert_parses(
        r#"handle: reopen(handle)
"#,
    );
}

#[test]
fn shadowing_transformed_value() {
    assert_parses(
        r#"value: transform(value)
"#,
    );
}

#[test]
fn tab_indented_place_rebinding() {
    assert_parses(
        r#"update():
	counter:: next
	point.x:: 10
	pair.(0):: value
	items.(index):: replacement
return 0
"#,
    );
}

#[test]
fn linear_handle_rebinding() {
    assert_parses(
        r#"handle:: reopen(handle)
"#,
    );
}
