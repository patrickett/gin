//! What the printer does to layout, case by case. Every case is also run through the
//! guarantees by `format`.

use ginfmt::{Align, Options, format, format_with};

fn same(source: &str) {
    assert_eq!(format(source).unwrap(), source);
}

fn becomes(source: &str, expected: &str) {
    assert_eq!(format(source).unwrap(), expected, "from {source:?}");
    same(expected);
}

#[test]
fn indentation_is_four_spaces_per_level_whatever_was_written() {
    becomes("run:\n  x: 1\n  when x < 2 then\n     return 0\n  return 1\n", "run:\n    x: 1\n    when x < 2 then\n        return 0\n    return 1\n");
    becomes("run:\n\tx: 1\n\treturn x\n", "run:\n    x: 1\n    return x\n");
}

#[test]
fn spacing_inside_a_line_is_normalised() {
    becomes("f(a   Int,b Int)  Int:  return a+b*2\n", "f(a Int, b Int) Int: return a + b * 2\n");
    becomes("f( a Int ) Int: return ( a )\n", "f(a Int) Int: return (a)\n");
    becomes("run:\n    self.position:: 0\n    buffer.[ 0 ]::1\n", "run:\n    self.position :: 0\n    buffer.[0] :: 1\n");
    becomes("x:   -1\ny: a - -1\nz: a-b\n", "x: -1\ny: a - -1\nz: a - b\n");
    becomes("x: #size( a )\ny: @ Byte\n", "x: #size(a)\ny: @Byte\n");
}

#[test]
fn compound_assignment_stays_one_operator() {
    same("run:\n    buffer.[0] +: 2\n");
}

#[test]
fn blank_lines_collapse_to_one_and_the_edges_are_trimmed() {
    becomes("\n\n\nuse core.io print_int\n\n\n\nrun:\n    return 0\n\n\n", "use core.io print_int\n\nrun:\n    return 0\n");
    becomes("a: 1\nb: 2\n", "a: 1\nb: 2\n");
    becomes("a: 1   \nb: 2\t\n", "a: 1\nb: 2\n");
    becomes("", "");
    becomes("   \n\n", "");
}

#[test]
fn a_missing_final_newline_is_added() {
    becomes("a: 1", "a: 1\n");
}

#[test]
fn comments_keep_their_text_and_follow_the_indentation_of_the_code() {
    becomes("run:\n        -- note\n    return 0   -- done  \n", "run:\n    -- note\n    return 0 -- done\n");
    becomes("--- doc\n--| module\n--text\nrun: return 0\n", "--- doc\n--| module\n--text\nrun: return 0\n");
    becomes("run:\n    return 0\n    -- last in the body\n-- top level again\nother: 1\n", "run:\n    return 0\n    -- last in the body\n-- top level again\nother: 1\n");
}

#[test]
fn strings_are_never_touched() {
    same("a: 'x   y -- not a comment'\nb: \"f  \\\" -- \\\\ z\"\n");
}

#[test]
fn field_types_align_within_a_run_of_adjacent_fields() {
    becomes(
        "Reader has\n    data @Byte\n    position SignedBigInt\n\n    available SignedBigInt\n    n Int\n",
        "Reader has\n    data     @Byte\n    position SignedBigInt\n\n    available SignedBigInt\n    n         Int\n",
    );
    // A comment, blank line or attribute ends the run.
    same("Box has\n    #private\n    allocator alloc\n    #private\n    storage Vector(element)\n");
    // Methods are not fields.
    same("Box has\n    value element\n    take(self) element: return self.value\n");
}

#[test]
fn field_alignment_can_be_off() {
    let source = "Reader has\n    data @Byte\n    position SignedBigInt\n";
    assert_eq!(format_with(source, Options { align: Align::Off, width: None }).unwrap(), source);
}

#[test]
fn continuation_lines_keep_their_line_breaks_and_move_one_level_in() {
    becomes(
        "Name is Red\n     or Blue\n     or Green\n",
        "Name is Red\n    or Blue\n    or Green\n",
    );
    becomes(
        "f(\n      a Int\n      b Int\n) Int:\n    return a\n",
        "f(\n    a Int\n    b Int\n) Int:\n    return a\n",
    );
    becomes(
        "run:\n    return when x < 1\n             then 1\n             else 2\n",
        "run:\n    return when x < 1\n        then 1\n        else 2\n",
    );
}

#[test]
fn long_lines_are_never_wrapped() {
    let long = format!("run: return {}\n", vec!["1"; 200].join(" + "));
    same(&long);
}

#[test]
fn a_flaw_is_reported_and_nothing_is_formatted() {
    let error = format("run:\n    return (\n").unwrap_err();
    assert!(matches!(error, ginfmt::FormatError::Flaws(ref d) if !d.is_empty()), "{error}");
    // Only flaws stop it: this unusual but valid input formats.
    assert!(format("a: 1\n").is_ok());
}

#[test]
fn wrapping_rules_at_100_columns() {
    let long = "\
Thing has Trait0WithALongishName and Trait1WithALongishName and Trait2WithALongishName and Trait3WithALongishName
Choice is Variant0WithALongishName or Variant1WithALongishName or Variant2WithALongishName or Variant3WithALongishName
run:
    result: when left_name_is_long < 1 then a_value_that_is_long_enough_to_wrap else another_value_that_is_long_enough_to_wrap_the_line
    return check(first_argument, condition_flag_0 = 1 and condition_flag_1 = 1 and condition_flag_2 = 1 and condition_flag_3 = 1, last_argument_name)
    return outer(inner(argument_0_with_a_longish_name, argument_1_with_a_longish_name), other(argument_2_with_a_longish_name, argument_3_with_a_longish_name))
";
    let wrapped = "\
Thing has Trait0WithALongishName
    and Trait1WithALongishName
    and Trait2WithALongishName
    and Trait3WithALongishName
Choice is Variant0WithALongishName
    or Variant1WithALongishName
    or Variant2WithALongishName
    or Variant3WithALongishName
run:
    result: when left_name_is_long < 1
        then a_value_that_is_long_enough_to_wrap
        else another_value_that_is_long_enough_to_wrap_the_line
    return check(
        first_argument
        condition_flag_0 = 1 and condition_flag_1 = 1 and condition_flag_2 = 1 and
            condition_flag_3 = 1
        last_argument_name
    )
    return outer(
        inner(argument_0_with_a_longish_name, argument_1_with_a_longish_name)
        other(argument_2_with_a_longish_name, argument_3_with_a_longish_name)
    )
";
    becomes(long, wrapped);
    // Arrays keep their commas.
    let array = format!("run:\n    items: [{}]\n", (0..6).map(|n| format!("element_number_{n}_long_name")).collect::<Vec<_>>().join(", "));
    let text = format(&array).unwrap();
    assert!(text.starts_with("run:\n    items: [\n        element_number_0_long_name,\n"), "{text}");
    assert!(text.ends_with("element_number_5_long_name\n    ]\n"), "{text}");
    // `where` claims and `use` members go one per line, claims one level in.
    let claims = format!("f(self mut) where {}:\n    return\n", (0..6).map(|n| format!("self.field_{n} :: self.field_{n} + 1")).collect::<Vec<_>>().join(", "));
    let text = format(&claims).unwrap();
    assert!(text.starts_with("f(self mut) where self.field_0 :: self.field_0 + 1,\n    self.field_1 :: self.field_1 + 1,\n"), "{text}");
    assert!(text.ends_with("self.field_5 :: self.field_5 + 1:\n    return\n"), "{text}");
    let members = format!("use core.x {}\n", (0..9).map(|n| format!("Member{n}WithALongName")).collect::<Vec<_>>().join(", "));
    let text = format(&members).unwrap();
    assert!(text.starts_with("use core.x Member0WithALongName,\n    Member1WithALongName,\n"), "{text}");
    // Result-variant claims break inside their parentheses.
    let variant = format!("f(self mut) Ok where ({}) or\n    Bad where (self :: self):\n    return\n", (0..6).map(|n| format!("self.field_{n} >= {n}000000")).collect::<Vec<_>>().join(", "));
    let text = format(&variant).unwrap();
    assert!(text.lines().all(|l| l.len() <= 100), "{text}");
    assert!(text.contains(",\n    self.field_"), "{text}");
}

#[test]
fn discriminants_line_up_their_equals_like_fields() {
    becomes(
        "ExitCode is Success   = 0\n         or FirstFlaw = 1\n         or SecFlaw   = 2\n",
        "ExitCode is Success = 0\n    or FirstFlaw    = 1\n    or SecFlaw      = 2\n",
    );
    becomes("LogLevel is Debug   =   'debug' or Info='info'\n", "LogLevel is Debug = 'debug' or Info = 'info'\n");
    // A comparison in a body is not a discriminant.
    same("run:\n    a: x = 1\n    b: yy = 2\n");
    let off = ginfmt::Options { align: Align::Off, width: None };
    assert_eq!(
        format_with("E is A = 0\n    or Bbb = 1\n", off).unwrap(),
        "E is A = 0\n    or Bbb = 1\n"
    );
}

#[test]
fn a_named_payload_field_keeps_its_name_and_its_type() {
    becomes(
        "Outcome is Fine or Failed(code   Code,detail Code)\n",
        "Outcome is Fine or Failed(code Code, detail Code)\n",
    );
    becomes(
        "make(n in 0 to 9) Made(span   Span) where (span.length = n) or Failed:\n    return Failed\n",
        "make(n in 0 to 9) Made(span Span) where (span.length = n) or Failed:\n    return Failed\n",
    );
}

#[test]
fn variant_payload_fields_keep_their_types() {
    same("Outcome(element) is Fine(value element) or Aborted(proposal in 1 to 16)\n");
    becomes(
        "Outcome(element) is Fine( value  element ) or Aborted(proposal   in 1 to 16,other Other)\n",
        "Outcome(element) is Fine(value element) or Aborted(proposal in 1 to 16, other Other)\n",
    );
    same("check(n in 0 to 9) Done(count in 0 to 9) or Failed(code element):\n    return Failed(n)\n");
}

#[test]
fn nested_patterns_keep_their_shape() {
    same("f(r Outcome) in 0 to 3: return when r is\n    Flaw(OutOfMemory) then 1\n    Done(Full(value)) then 2\n");
    becomes(
        "f(r Outcome) in 0 to 3: return when r is\n    Flaw( OutOfMemory ) then 1\n    Done(Full( value )) then 2\n",
        "f(r Outcome) in 0 to 3: return when r is\n    Flaw(OutOfMemory) then 1\n    Done(Full(value)) then 2\n",
    );
}

#[test]
fn a_nested_pattern_keeps_its_shape() {
    becomes(
        "show(step Step) Code: return when step is\n    Done( Full( value ) ) then value\n    Done(Empty) then 1\n",
        "show(step Step) Code: return when step is\n    Done(Full(value)) then value\n    Done(Empty) then 1\n",
    );
    becomes(
        "f(s Step) Code: return when s is\n    Done(t :tag,Full( v ):slot) then v\n",
        "f(s Step) Code: return when s is\n    Done(t: tag, Full(v): slot) then v\n",
    );
}

#[test]
fn a_guarded_nested_arm_and_a_depth_three_pattern_round_trip() {
    // Audit 2026-11-05: the guard stays after the pattern, a deep pattern keeps every level, and
    // the labeled form keeps the pattern first. `becomes` also checks the result is stable.
    becomes(
        "f(s Step, n in 0 to 9) Code: return when s is\n    Pending then 0\n    Done( Full( High( v ) ) )   and  n>3 then 7\n    Done(Full(High(v))) then v\n    Done(Full(Low)) then 1\n    Done(Empty) then 2\n",
        "f(s Step, n in 0 to 9) Code: return when s is\n    Pending then 0\n    Done(Full(High(v))) and n > 3 then 7\n    Done(Full(High(v))) then v\n    Done(Full(Low)) then 1\n    Done(Empty) then 2\n",
    );
    becomes(
        "f(s Step) Code: return when s is\n    Done(Full( v ):slot,_ :tag) and v>1 then v\n    Done(Empty:slot,_:tag) then 0\n",
        "f(s Step) Code: return when s is\n    Done(Full(v): slot, _: tag) and v > 1 then v\n    Done(Empty: slot, _: tag) then 0\n",
    );
}

#[test]
fn the_field_first_label_in_a_pattern_is_a_flaw_and_nothing_is_formatted() {
    // Negative control for the above: `Done(slot: Full(v))` is a construction, not a pattern, so
    // in an arm it is a parse error and the formatter must refuse rather than guess.
    let source = "f(s Step) Code: return when s is\n    Done(slot: Full(v), _: tag) then v\n";
    let error = format(source).unwrap_err();
    assert!(format!("{error:?}").contains("in a pattern the pattern comes first"), "{error:?}");
}
