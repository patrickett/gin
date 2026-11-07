//! `Bool is True or False`: a union declares its own payload-less variants.
//!
//! A bare capitalized name in an `or` list is a new variant with no fields, owned by the
//! union. No `True has` block exists anywhere. An `or` list never refers to an existing type:
//! a type is carried by wrapping it (`Count(Count)`), and a bare name that matches a type in
//! scope is refused.
use typecheck::{
    AggregateKind, Capability, CheckedSourceBodyKind, CheckedSourceModule, Limits,
    SourceCheckError, check_module,
};

const BOOL: &str = "\
#unrestricted
Bool is True or False

to_number(flag Bool) in 0 to 1: when flag is
    True then 1
    False then 0
";

fn module(text: &str) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module("case.union", &source, &[], Limits::default())
}

fn rejects(text: &str, expected: SourceCheckError) {
    assert_eq!(module(text).err(), Some(expected), "{text}");
}

#[test]
fn a_union_declares_two_variants_with_no_payload() {
    let module = module(BOOL).unwrap();
    let file = module.checked();
    let schema = |name: &str| file.aggregates.iter().find(|a| a.name == name).unwrap();
    let bool_ = schema("Bool");
    assert_eq!(bool_.kind, AggregateKind::Sum);
    assert_eq!(bool_.variants.len(), 2);
    for name in ["Bool.True", "Bool.False"] {
        let variant = schema(name);
        assert_eq!(variant.kind, AggregateKind::Record, "{name}");
        assert!(variant.fields.is_empty(), "{name} holds no payload");
        assert_eq!(
            variant.capability,
            Capability::Unrestricted,
            "{name} owns nothing"
        );
    }
}

#[test]
fn the_variants_line_up_vertically() {
    let text = "\
#unrestricted
Bool is True
     or False

to_number(flag Bool) in 0 to 1: when flag is
    True then 1
    False then 0
";
    module(text).unwrap();
}

#[test]
fn a_wrapped_type_is_carried_and_a_bare_name_is_a_variant() {
    let text = "\
#unrestricted
Count has
    value in 0 to 9

Outcome is Count(Count) or Failed

describe(outcome Outcome) in 0 to 9: when outcome is
    Count(count) then count.value
    Failed then 0
";
    let module = module(text).unwrap();
    let file = module.checked();
    let schema = |name: &str| file.aggregates.iter().find(|a| a.name == name).unwrap();
    assert_eq!(
        schema("Count").fields.len(),
        1,
        "the declared type is untouched"
    );
    assert!(schema("Outcome.Failed").fields.is_empty());
    assert_eq!(schema("Outcome.Count").fields.len(), 1, "the wrapper carries the type");
    assert_eq!(schema("Outcome").variants.len(), 2);
    // A bare name that matches a type in scope is no longer a reference to it.
    rejects(
        "#unrestricted\nCount has\n    value in 0 to 9\n\nOutcome is Count or Failed\n",
        SourceCheckError::Type,
    );
}

#[test]
fn a_variant_is_a_value() {
    let text =
        format!("{BOOL}yes() Bool: return True\nanswer() in 0 to 1: return to_number(yes())\n");
    module(&text).unwrap();
}

#[test]
fn a_known_variant_selects_its_arm_at_compile_time() {
    let text = format!("{BOOL}answer() in 0 to 1: return to_number(True)\n");
    let module = module(&text).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "answer")
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    assert!(
        body.expressions
            .iter()
            .all(|expr| !matches!(expr.kind, CheckedSourceBodyKind::Call { .. })),
        "{:?}",
        body.expressions
    );
    assert!(body.expressions.iter().any(
        |expr| matches!(&expr.kind, CheckedSourceBodyKind::Integer(value) if *value == 1.into())
    ));
}

#[test]
fn every_variant_needs_an_arm() {
    rejects(
        "#unrestricted\nBool is True or False\nf(flag Bool) in 0 to 1: when flag is\n    True then 1\n",
        SourceCheckError::Type,
    );
}

#[test]
fn an_arm_naming_no_variant_is_unknown() {
    rejects(
        "#unrestricted\nBool is True or False\nf(flag Bool) in 0 to 1: when flag is\n    True then 1\n    Maybe then 0\n",
        SourceCheckError::UnknownName,
    );
}

/// Only a capitalized name can be a variant: a lowercase one stays an unknown name.
#[test]
fn a_lowercase_name_is_not_a_variant() {
    // Refused as a malformed `or` list entry (`Type`): a variant is a capitalized name.
    rejects("Bool is true or false\n", SourceCheckError::Type);
}

const COLORS: &str = "\
#unrestricted
Color is Red or Blue or Green

#unrestricted
Name is Red or Blue or Green

color_number(color Color) in 0 to 2: when color is
    Red then 0
    Blue then 1
    Green then 2

name_number(name Name) in 0 to 2: when name is
    Red then 0
    Blue then 1
    Green then 2
";

/// A variant belongs to its union: `Name.Red` is not `Color.Red`.
#[test]
fn two_unions_may_declare_the_same_variant_names() {
    let module = module(COLORS).unwrap();
    let file = module.checked();
    let names: Vec<_> = file.aggregates.iter().map(|a| a.name.as_str()).collect();
    for expected in ["Color.Red", "Name.Red", "Color.Green", "Name.Green"] {
        assert!(names.contains(&expected), "{names:?}");
    }
    let schema = |name: &str| file.aggregates.iter().position(|a| a.name == name).unwrap();
    assert_ne!(schema("Color.Red"), schema("Name.Red"));
}

#[test]
fn a_variant_of_one_union_is_not_a_variant_of_another() {
    let text = format!("{COLORS}wrong() in 0 to 2: return color_number(Name.Red)\n");
    rejects(&text, SourceCheckError::Type);
    let text = format!("{COLORS}right() in 0 to 2: return color_number(Color.Red)\n");
    module(&text).unwrap();
    let text = format!("{COLORS}right() in 0 to 2: return name_number(Name.Red)\n");
    module(&text).unwrap();
}

#[test]
fn a_qualified_variant_is_a_value() {
    let text = format!(
        "{COLORS}make() Name: return Name.Green\n\
         both() in 0 to 4: return name_number(make()) + name_number(Name.Blue)\n"
    );
    module(&text).unwrap();
}

/// A variant carries nothing, so there is nothing to call.
#[test]
fn a_variant_is_not_called() {
    let text = format!("{COLORS}make() Name: return Name.Green()\n");
    rejects(&text, SourceCheckError::Type);
}

/// A bare variant name means the variant of the type that is expected there.
#[test]
fn a_bare_variant_resolves_by_the_expected_type() {
    for text in [
        format!("{COLORS}a() in 0 to 2: return color_number(Red)\n"),
        format!("{COLORS}b() in 0 to 2: return name_number(Red)\n"),
        format!("{COLORS}c() Color: return Blue\n"),
    ] {
        module(&text).unwrap();
    }
    let text = format!("{COLORS}d() in 0 to 2:\n    x := Red\n    return color_number(x)\n");
    rejects(&text, SourceCheckError::UnknownName);
}

#[test]
fn a_pattern_names_a_variant_of_its_subject_only() {
    let text = "\
#unrestricted
Color is Red or Blue

#unrestricted
Shade is Light or Dark

f(color Color) in 0 to 1: when color is
    Red then 0
    Light then 1
";
    // `Light` belongs to `Shade`; outside it the name means nothing.
    rejects(text, SourceCheckError::UnknownName);
}

#[test]
fn a_qualified_variant_folds_at_compile_time() {
    let text = format!("{COLORS}pick() in 0 to 2: return color_number(Color.Blue)\n");
    let module = module(&text).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "pick")
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    assert!(body.expressions.iter().all(|expr| !matches!(
        expr.kind,
        CheckedSourceBodyKind::Call { .. } | CheckedSourceBodyKind::Match { .. }
    )));
    assert!(body.expressions.iter().any(
        |expr| matches!(&expr.kind, CheckedSourceBodyKind::Integer(value) if *value == 1.into())
    ));
}

/// A client never names a variant's module: the variants come with the union.
#[test]
fn an_imported_union_brings_its_variants() {
    let library = module(BOOL).unwrap();
    let client = "\
use case.union Bool, to_number

yes() Bool: return Bool.True
no() Bool: return False

flip(flag Bool) Bool: when flag is
    True then Bool.False
    False then Bool.True

answer() in 0 to 1: return to_number(flip(yes()))
";
    let source = parser::parse_strict(client).unwrap();
    let checked = check_module("case.client", &source, &[&library], Limits::default());
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_declared_type_may_be_wrapped_by_every_union_that_carries_it() {
    let text = "\
Red has

Color is Red(Red) or Blue

Name is Red(Red) or Green
";
    let module = module(text).unwrap();
    let file = module.checked();
    let names: Vec<_> = file.aggregates.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"Red"), "{names:?}");
    for expected in ["Color.Red", "Color.Blue", "Name.Red", "Name.Green"] {
        assert!(names.contains(&expected), "{names:?}");
    }
    let schema = |name: &str| file.aggregates.iter().find(|a| a.name == name).unwrap();
    assert_eq!(schema("Color.Red").fields.len(), 1, "the wrapper carries the one Red");
    assert_eq!(schema("Name.Red").fields.len(), 1);
}

#[test]
fn a_union_wraps_a_declared_type_to_carry_it() {
    module("Red has\nLight is Red(Red) or Green\nSignal is Red(Red) or Stop\n").unwrap();
    // The bare name would have been the type: refused, with the wrapped form as the way out.
    rejects("Red has\nLight is Red or Green\n", SourceCheckError::Type);
}

#[test]
fn a_variant_cannot_repeat_inside_one_union() {
    rejects("Bool is True or True\n", SourceCheckError::Duplicate);
}

/// A name that resolves to a type declared later is still that type, wherever it is written:
/// bare it is refused, wrapped it carries the type.
#[test]
fn a_declaration_after_the_union_is_still_the_type_it_names() {
    rejects("Bool is True or False\nTrue has\n    value in 0 to 1\n", SourceCheckError::Type);
    let module = module("Bool is True(True) or False\nTrue has\n    value in 0 to 1\n").unwrap();
    let file = module.checked();
    let schema = |name: &str| file.aggregates.iter().find(|a| a.name == name).unwrap();
    assert_eq!(schema("True").fields.len(), 1);
    assert!(schema("Bool.False").fields.is_empty());
}

/// A value that may be copied and discarded may also be ignored: an `#unrestricted` union
/// is no different from an `#unrestricted` record in that.
#[test]
fn an_unrestricted_union_parameter_may_be_ignored() {
    let text = "#unrestricted\nBool is True or False\nignore(flag Bool) in 0 to 0: return 0\n";
    module(text).unwrap();
    let record = "#unrestricted\nPair has first in 0 to 1\nignore(pair Pair) in 0 to 0: return 0\n";
    module(record).unwrap();
}

/// A linear union must be consumed, so ignoring one is refused. A union is linear unless it is
/// marked `#unrestricted`, whatever its variants carry.
#[test]
fn a_linear_union_parameter_may_not_be_ignored() {
    let text = "Token has n in 0 to 1\nFlag is Held(Token) or Free\nignore(flag Flag) in 0 to 0: return 0\n";
    assert!(module(text).is_err());
    let plain = "Bool is True or False\nignore(flag Bool) in 0 to 0: return 0\n";
    assert!(module(plain).is_err());
}

#[test]
fn an_ignored_unrestricted_union_may_be_passed_on_and_dropped() {
    let text = "\
#unrestricted
Bool is True or False

first(flag Bool) in 0 to 0: return 0

second(flag Bool) in 0 to 0: return first(flag)
";
    module(text).unwrap();
}

/// One union reaching a module along two paths is still one union: its variants are matched
/// by where they were declared, not by the path they arrived on.
#[test]
fn a_union_imported_along_two_paths_is_one_type() {
    let base = module("#unrestricted\nMode is Loud or Quiet\n\nflip(mode Mode) Mode: when mode is\n    Loud then Mode.Quiet\n    Quiet then Mode.Loud\n")
        .unwrap();
    let check = |name: &str, text: &str, dependencies: &[&CheckedSourceModule]| {
        let source = parser::parse_strict(text).unwrap();
        check_module(name, &source, dependencies, Limits::default())
    };
    let left = check(
        "case.left",
        "use case.union Mode, flip\nmake_loud() Mode: return Mode.Loud\n",
        &[&base],
    )
    .unwrap();
    let right = check(
        "case.right",
        "use case.union Mode, flip\nmake_quiet() Mode: return Mode.Quiet\n",
        &[&base],
    )
    .unwrap();
    let joined = check(
        "case.joined",
        "use case.union Mode, flip\nuse case.left make_loud\nuse case.right make_quiet\n\
         describe(mode Mode) in 0 to 1: when mode is\n    Loud then 1\n    Quiet then 0\n\
         answer() in 0 to 1: return describe(flip(make_loud())) + describe(make_quiet())\n",
        &[&base, &left, &right],
    );
    assert!(joined.is_ok(), "{:?}", joined.err());
}
