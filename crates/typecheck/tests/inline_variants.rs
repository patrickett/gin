//! A result written as variants declares the variants as its own.
use typecheck::{Limits, SourceCheckError, check_file_diagnostic};

fn run(text: &str) -> Result<(), typecheck::SourceCheckFailure> {
    let file = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    check_file_diagnostic(&file, Limits::default()).map(|_| ())
}

fn accepts(text: &str) {
    if let Err(failure) = run(text) {
        panic!("{text}\n{failure:?}");
    }
}

fn refuses(text: &str, error: SourceCheckError) -> String {
    let failure = run(text).expect_err(text);
    assert_eq!(failure.error, error, "{text}\n{failure:?}");
    failure.detail.unwrap_or_default()
}

const CODE: &str = "#unrestricted\nCode is in 0 to 9\n\n";

#[test]
fn a_header_declares_its_own_variants_and_the_caller_matches_them() {
    accepts(&format!(
        "{CODE}check(n in 0 to 9) Fine or Failed(Code):\n    when n < 5 then\n        return Fine\n    return Failed(n as Code)\n\nrank(n in 0 to 9) in 0 to 1: when check(n) is\n    Fine then 0\n    Failed(_) then 1\n"
    ));
}

#[test]
fn a_variant_may_share_a_types_name_only_when_it_wraps() {
    accepts(&format!(
        "{CODE}check(n in 0 to 9) Fine or Code(Code):\n    when n < 5 then\n        return Fine\n    return Code(n as Code)\n"
    ));
    let detail = refuses(
        &format!("{CODE}check(n in 0 to 9) Fine or Code:\n    return Fine\n"),
        SourceCheckError::Type,
    );
    assert!(detail.contains("`Code(Code)`"), "{detail}");
}

#[test]
fn a_returned_variant_must_be_one_of_the_headers() {
    refuses(
        &format!("{CODE}check(n in 0 to 9) Fine or Failed(Code):\n    return Other\n"),
        SourceCheckError::UnknownName,
    );
}

#[test]
fn a_duplicate_variant_is_refused() {
    refuses(
        &format!("{CODE}check(n in 0 to 9) Fine or Fine:\n    return Fine\n"),
        SourceCheckError::Duplicate,
    );
}

#[test]
fn the_variants_are_not_names_of_the_module() {
    // `Fine` is the function's variant, not a value of the module.
    refuses(
        &format!("{CODE}check(n in 0 to 9) Fine or Failed(Code):\n    return Fine\n\nuse_it: return Fine\n"),
        SourceCheckError::UnknownName,
    );
}

#[test]
fn claims_of_the_returned_variant_are_proved() {
    let source = |claim: &str| {
        format!(
            "{CODE}check(n in 0 to 9) Fine where ({claim}) or Failed(Code):\n    when n < 5 then\n        return Fine\n    return Failed(n as Code)\n"
        )
    };
    accepts(&source("n < 5"));
}

#[test]
fn a_claim_the_returned_variant_does_not_meet_is_refused() {
    let text = format!(
        "{CODE}check(n in 0 to 9) Fine where (n < 3) or Failed(Code):\n    when n < 5 then\n        return Fine\n    return Failed(n as Code)\n"
    );
    let failure = run(&text).expect_err("n < 5 does not give n < 3");
    assert!(
        matches!(failure.error, SourceCheckError::Unproved | SourceCheckError::Type),
        "{failure:?}"
    );
}

// Slice 2: a generic function. The variants range over its type parameters.

const BOXED: &str = "\
#unrestricted
Cell has
    item in 0 to 9

wrap(element, value element) Made(element) or Empty:
    return Made(value)

take(cell Cell) in 0 to 9: when wrap(Cell, cell) is
    Made(inner) then 1
    Empty then 0
";

#[test]
fn a_generic_function_declares_variants_over_its_type_parameters() {
    accepts(BOXED);
}

#[test]
fn a_payload_type_must_resolve_in_the_functions_scope() {
    refuses(
        "wrap(element, value element) Made(Missing) or Empty:\n    return Empty\n",
        SourceCheckError::UnknownName,
    );
}

// Slice 3: across modules. An importer receives the sum and matches it by variant name.

fn module(
    name: &str,
    text: &str,
    dependencies: &[&typecheck::CheckedSourceModule],
) -> Result<typecheck::CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    typecheck::check_module(name, &source, dependencies, Limits::default())
}

const LIBRARY: &str = "\
#unrestricted
Code is in 0 to 9

check(n in 0 to 9) Fine or Failed(Code):
    when n < 5 then
        return Fine
    return Failed(n as Code)

wrap(element, value element) Made(element) or Empty:
    return Made(value)
";

#[test]
fn an_importer_matches_the_variants_of_an_imported_inline_result() {
    let library = module("demo.checks", LIBRARY, &[]).expect("library");
    let client = "\
use demo.checks check

rank(n in 0 to 9) in 0 to 1: when check(n) is
    Fine then 0
    Failed(_) then 1
";
    module("app.client", client, &[&library]).unwrap_or_else(|e| panic!("{e:?}"));
    // Negative control: a variant the function does not declare.
    let wrong = "\
use demo.checks check

rank(n in 0 to 9) in 0 to 1: when check(n) is
    Fine then 0
    Missing then 1
";
    assert!(module("app.wrong", wrong, &[&library]).is_err());
    // A guard on the imported result, then use of the rest.
    let guard = "\
use demo.checks check

rank(n in 0 to 9) in 0 to 1:
    got: check(n)
    when got is Failed(_) then
        return 1
    return 0
";
    module("app.guard", guard, &[&library]).unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn an_imported_generic_inline_result_is_instantiated_by_the_importer() {
    let library = module("demo.checks", LIBRARY, &[]).expect("library");
    let client = "\
use demo.checks wrap

#unrestricted
Cell has
    item in 0 to 9

take(cell Cell) in 0 to 1: when wrap(Cell, cell) is
    Made(inner) then 1
    Empty then 0
";
    module("app.generic", client, &[&library]).unwrap_or_else(|e| panic!("{e:?}"));
}

// Slice 5: the claims of each variant are proved where that variant is returned, and only there.

const CLAIMED: &str = "\
#unrestricted
Code is in 0 to 9

check(n in 0 to 9) Fine where (n < 5) or Failed(Code) where (n >= 5):
    when n < 5 then
        return Fine
    return Failed(n as Code)

";

#[test]
fn each_variants_claims_are_proved_at_its_own_return() {
    accepts(&format!("{CLAIMED}use_it(n in 0 to 9) in 0 to 1: when check(n) is\n    Fine then 0\n    Failed(_) then 1\n"));
    // Negative controls: a claim that is false where its variant is returned is refused, for
    // either variant.
    for wrong in [
        "check(n in 0 to 9) Fine where (n < 5) or Failed(Code) where (n >= 7):",
        "check(n in 0 to 9) Fine where (n < 3) or Failed(Code) where (n >= 5):",
    ] {
        let text = format!(
            "#unrestricted\nCode is in 0 to 9\n\n{wrong}\n    when n < 5 then\n        return Fine\n    return Failed(n as Code)\n"
        );
        let failure = run(&text).expect_err(&text);
        assert!(matches!(failure.error, SourceCheckError::Unproved | SourceCheckError::Type), "{failure:?}");
    }
}

// Named payload fields: `Made(span Span)` names the payload, so a claim can refer to it.

const SPAN: &str = "\
#unrestricted
Span has
    start in 0 to 9
    length in 0 to 9

";

fn make(header: &str, body: &str) -> String {
    format!("{SPAN}{header}:\n{body}")
}

const MAKE_BODY: &str = "    when n > 5 then\n        return Failed\n    return Made(Span(start: 0, length: n))\n";

#[test]
fn a_claim_over_a_named_payload_is_proved_and_assumed_per_variant() {
    let source = format!(
        "{}\nlength_of(n in 0 to 3) in 0 to 3: when make(n) is\n    Made(found) then found.length\n    Failed then 0\n",
        make("make(n in 0 to 9) Made(span Span) where (span.length = n) or Failed", MAKE_BODY)
    );
    accepts(&source);
    // Negative controls. The claim is false where the variant is returned:
    let wrong = make("make(n in 0 to 9) Made(span Span) where (span.length = 0) or Failed", MAKE_BODY);
    assert!(run(&wrong).is_err(), "span.length = 0 does not hold");
    // An importer assumes the claim of the variant it matched.
    let client = "\
use demo.spans make

length_of(n in 0 to 3) in 0 to 3: when make(n) is
    Made(found) then found.length
    Failed then 0
";
    let with_claim = module(
        "demo.spans",
        &make("make(n in 0 to 9) Made(span Span) where (span.length = n) or Failed", MAKE_BODY),
        &[],
    )
    .expect("library");
    module("app.client", client, &[&with_claim]).unwrap_or_else(|e| panic!("{e:?}"));
    // (An importer also reads the summaries of the body, which here say the same as the claim, so
    // the claim's own effect is checked by the false-claim control above, not by dropping it.)
}

#[test]
fn an_unnamed_payload_cannot_be_named_by_a_claim() {
    let text = make("make(n in 0 to 9) Made(Span) where (span.length = n) or Failed", MAKE_BODY);
    let failure = run(&text).expect_err("no name for the payload");
    assert_eq!(failure.error, SourceCheckError::UnknownName, "{failure:?}");
}

#[test]
fn field_names_are_unique_and_do_not_shadow_parameters() {
    refuses(&make("make(n in 0 to 9) Made(a Span, a Span) or Failed", "    return Failed\n"), SourceCheckError::Duplicate);
    let detail = refuses(
        &make("make(n in 0 to 9) Made(n Span) where (n.length = 1) or Failed", MAKE_BODY),
        SourceCheckError::Duplicate,
    );
    assert!(detail.contains("name of a parameter"), "{detail}");
}

#[test]
fn several_payloads_are_built_by_label_and_matched_by_position_or_label() {
    let header = "make(n in 0 to 9) Pair(first Span, second Span) or Failed";
    let body = "    when n > 5 then\n        return Failed\n    return Pair(first: Span(start: 0, length: n), second: Span(start: 1, length: n))\n";
    let positional = format!(
        "{}\nuse_it(n in 0 to 9) in 0 to 9: when make(n) is\n    Pair(a, b) then a.length\n    Failed then 0\n",
        make(header, body)
    );
    accepts(&positional);
    let labeled = format!(
        "{}\nuse_it(n in 0 to 9) in 0 to 9: when make(n) is\n    Pair(x: first, y: second) then y.length\n    Failed then 0\n",
        make(header, body)
    );
    accepts(&labeled);
    // Negative controls: a label the variant does not have, and a wrong number of binders.
    let wrong = format!(
        "{}\nuse_it(n in 0 to 9) in 0 to 9: when make(n) is\n    Pair(x: third, y: second) then y.length\n    Failed then 0\n",
        make(header, body)
    );
    assert!(run(&wrong).is_err());
    let few = format!(
        "{}\nuse_it(n in 0 to 9) in 0 to 9: when make(n) is\n    Pair(a) then 1\n    Failed then 0\n",
        make(header, body)
    );
    assert!(run(&few).is_err());
}

#[test]
fn a_first_payload_may_be_given_without_its_label() {
    let text = format!(
        "{}\nuse_it(n in 0 to 9) in 0 to 9: when make(n) is\n    Made(found) then found.length\n    Failed then 0\n",
        make(
            "make(n in 0 to 9) Made(span Span) or Failed",
            "    when n > 5 then\n        return Failed\n    return Made(Span(start: 0, length: n))\n"
        )
    );
    accepts(&text);
}

// The caller assumes the claims of the variant it matched, including those over inputs.

// The claims of the matched variant are kept with the result and assumed where that variant is in
// play: in its arm, or in the detached check of an arm no world selects, never in the other arms.
#[test]
fn the_caller_assumes_a_claim_over_an_input_in_the_matching_arm() {
    let checks = format!("{CLAIMED}");
    accepts(&format!(
        "{checks}low(n in 0 to 9) in 0 to 4: when check(n) is\n    Fine then n\n    Failed(_) then 0\n"
    ));
    // Negative control: in the other arm the claim of `Fine` is not assumed.
    assert!(run(&format!(
        "{checks}low(n in 0 to 9) in 0 to 4: when check(n) is\n    Fine then 0\n    Failed(_) then n\n"
    ))
    .is_err());
}

#[test]
fn a_union_declares_named_payload_fields() {
    accepts(
        "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nOutcome is Fine or Failed(code Code, detail Code)\n\nrank(o Outcome) in 0 to 1: when o is\n    Fine then 0\n    Failed(a, b) then 1\n",
    );
    let failure = run(
        "#unrestricted\nCode is in 0 to 9\n\n#unrestricted\nOutcome is Fine or Failed(code Code, code Code)\n",
    )
    .expect_err("duplicate field");
    assert_eq!(failure.error, SourceCheckError::Duplicate);
}

// Trait-provided methods with inline own-variant results.

const TRAIT: &str = "\
#unrestricted
Code is in 0 to 9

Maker has
    make(self, n in 0 to 9) Fine or Failed(Code)

#unrestricted
Plain has Maker
    n in 0 to 9

    make(self, n in 0 to 9) Fine or Failed(Code):
        when n < 5 then
            return Fine
        return Failed(n as Code)

rank(kind has Maker, m kind, n in 0 to 9) in 0 to 1: when m.make(n) is
    Fine then 0
    Failed(_) then 1

#entry
main in 0 to 1:
    return rank(Plain, Plain(n: 1), 7)
";

#[test]
fn a_provider_of_a_trait_with_an_inline_result_conforms_to_it() {
    accepts(TRAIT);
}

#[test]
fn a_call_through_a_trait_bound_matches_an_inline_own_variant_result() {
    let text = "\
#unrestricted
Code is in 0 to 9

Maker has
    make(self, n in 0 to 9) Fine or Failed(Code)

rank(kind has Maker, m kind, n in 0 to 9) in 0 to 1: when m.make(n) is
    Fine then 0
    Failed(_) then 1
";
    accepts(text);
}

fn provider(header: &str, extra: &str) -> String {
    format!(
        "#unrestricted\nCode is in 0 to 9\n\nMaker has\n    make(self, n in 0 to 9) Fine or Failed(Code) where (n >= 5)\n\n#unrestricted\nPlain has Maker\n    k in 0 to 9\n\n    {header}:\n        return Fine\n{extra}"
    )
}

#[test]
fn a_provider_must_repeat_the_traits_variants_and_claims() {
    // The same header conforms.
    accepts(&provider("make(self, n in 0 to 9) Fine or Failed(Code) where (n >= 5)", ""));
    // Negative controls, each refused as `Type` (a mismatch with the trait), none as a crash.
    for wrong in [
        "make(self, n in 0 to 9) Fine or Broken(Code) where (n >= 5)",    // wrong variant name
        "make(self, n in 0 to 9) Fine or Failed(Plain) where (n >= 5)",   // wrong payload
        "make(self, n in 0 to 9) Fine",                                    // missing variant
        "make(self, n in 0 to 9) Failed(Code) or Fine",                    // different order
        "make(self, n in 0 to 9) Fine or Failed(Code) where (n >= 6)",     // claim mismatch
        "make(self, n in 0 to 9) Fine or Failed(Code) or Extra",           // extra variant
    ] {
        let failure = run(&provider(wrong, "")).expect_err(wrong);
        assert!(
            matches!(failure.error, SourceCheckError::Type | SourceCheckError::MissingBody),
            "{wrong}: {failure:?}"
        );
    }
}

#[test]
fn a_generic_provider_gets_the_trait_sum_over_its_own_parameters() {
    let text = "\
#unrestricted
Code is in 0 to 9

Maker has
    make(self, element, value element) Made(element) or Empty

#unrestricted
Plain has Maker
    k in 0 to 9

    make(self, element, value element) Made(element) or Empty:
        return Made(value)

use_it(p Plain, flag in 0 to 1) in 0 to 1: when p.make(Code, 3 as Code) is
    Made(_) then 0
    Empty then 1
";
    accepts(text);
    // Negative control: a provider that does not write the trait's type parameter.
    let wrong = text.replace("    make(self, element, value element) Made(element) or Empty:\n        return Made(value)", "    make(self, value Code) Made(Code) or Empty:\n        return Empty");
    assert!(run(&wrong).is_err());
}

#[test]
fn a_provider_in_another_module_matches_the_traits_variants() {
    let library = module(
        "demo.maker",
        "#unrestricted\nCode is in 0 to 9\n\nMaker has\n    make(self, n in 0 to 9) Fine or Failed(Code)\n\nrank(kind has Maker, m kind, n in 0 to 9) in 0 to 1: when m.make(n) is\n    Fine then 0\n    Failed(_) then 1\n",
        &[],
    )
    .expect("library");
    let client = "\
use demo.maker Maker, Code, rank

#unrestricted
Plain has Maker
    k in 0 to 9

    make(self, n in 0 to 9) Fine or Failed(Code):
        when n < 5 then
            return Fine
        return Failed(n as Code)

#entry
main in 0 to 1:
    return rank(Plain, Plain(k: 1), 7)
";
    let source = parser::parse_strict(client).unwrap();
    let result = typecheck::check_module_diagnostic(
        "app.client",
        &source,
        &[&library],
        Limits::default(),
    );
    if let Err(failure) = result {
        panic!("{failure:?}");
    }
}

#[test]
fn a_variant_field_may_be_typed_by_a_type_parameter_or_a_range() {
    accepts(
        "#unrestricted\nOutcome(element) is Fine(value element) or Aborted(proposal in 1 to 16)\n\nwrap(element Type, value element) Outcome(element): return Fine(value)\n\nabort(element Type) Outcome(element): return Aborted(3)\n\nread(outcome Outcome(Code)) in 0 to 16: when outcome is\n    Fine(v) then 1\n    Aborted(p) then p\n\n#unrestricted\nCode is in 0 to 9\n",
    );
    // Negative controls: a value outside the range, an empty range.
    refuses(
        "#unrestricted\nOutcome is Fine or Aborted(proposal in 1 to 16)\n\nabort() Outcome: return Aborted(30)\n",
        SourceCheckError::Type,
    );
    refuses(
        "#unrestricted\nOutcome is Fine or Aborted(proposal in 16 to 1)\n",
        SourceCheckError::Type,
    );
    // Inline results declare them too.
    accepts("check(n in 0 to 9) Fine(count in 0 to 9) or Failed(code in 1 to 3):\n    return Fine(n)\n");
}

#[test]
fn a_trait_with_parameters_gives_its_inline_result_the_provisions_arguments() {
    let text = "\
#unrestricted
Code is in 0 to 9

#unrestricted
Wide is in 0 to 99

Maker(payload) has
    make(self, value payload) Made(payload) or Empty

#unrestricted
Plain has Maker(Code)
    k in 0 to 9

    make(self, value Code) Made(Code) or Empty:
        return Made(value)

use_it(p Plain) in 0 to 9: return when p.make(3 as Code) is
    Made(code) then code
    Empty then 0

rank(kind has Maker(Code), m kind) in 0 to 9: return when m.make(4 as Code) is
    Made(code) then code
    Empty then 0
";
    accepts(text);
    // Negative controls: a payload other than the provision's argument, and a provision that
    // does not give the trait its parameter.
    let wrong = text.replace("make(self, value Code) Made(Code) or Empty:", "make(self, value Code) Made(Wide) or Empty:");
    assert!(run(&wrong).is_err());
    let bare = text.replace("Plain has Maker(Code)", "Plain has Maker");
    assert!(run(&bare).is_err());
}
