use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::transform_file;
use typecheck::transform::{PackageTransformOptions, transform_package_with_shared_context};
use typecheck::ty::Ty;
use typecheck::{FileId, TagId};

fn transformed(source: &str) -> typecheck::TypedFileAst {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    transform_file(output.ast, FileId(0))
}

fn field_type(typed: &typecheck::TypedFileAst, field: &str) -> Ty {
    let holder = typed
        .tags
        .get(&TagId(Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = typed
        .type_registry
        .resolved_definition_for_type(&holder.resolved_ty)
    else {
        panic!("Holder record");
    };
    fields
        .iter()
        .find(|(name, _)| name.as_str() == field)
        .map(|(_, ty)| (**ty).clone())
        .expect("field")
}

#[test]
fn count_applications_with_different_units_have_distinct_identity() {
    let typed = transformed(
        "Byte is in 0...255\nCodePoint is in 0...1114111\n#phantom(unit)\nCount(unit) is in 0...255\nHolder has bytes Count(Byte), characters Count(CodePoint)\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let bytes = field_type(&typed, "bytes");
    let characters = field_type(&typed, "characters");
    assert_ne!(bytes, characters);
    assert_eq!(bytes.type_id(), characters.type_id());
    assert_ne!(
        bytes.named_instance().expect("Count(Byte)").arguments,
        characters
            .named_instance()
            .expect("Count(CodePoint)")
            .arguments
    );
    let count = typed
        .tags
        .get(&TagId(Intern::from_ref("Count")))
        .expect("Count");
    assert_eq!(
        count.attributes.phantom_parameters(),
        vec![Intern::from_ref("unit")]
    );
    assert_eq!(
        typecheck::representation::Repr::derive(&bytes, Some(&typed.type_registry)),
        typecheck::representation::Repr::derive(&characters, Some(&typed.type_registry))
    );
}

#[test]
fn transparent_unit_alias_reuses_applied_nominal_identity() {
    let typed = transformed(
        "Byte is in 0...255\n#phantom(unit)\nCount(unit) is in 0...255\nByteCount is Count(Byte)\nHolder has direct Count(Byte), alias ByteCount\n",
    );
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(field_type(&typed, "direct"), field_type(&typed, "alias"));
}

#[test]
fn bare_generic_integer_application_is_rejected() {
    let typed = transformed("#phantom(unit)\nCount(unit) is in 0...255\nvalue Count\n");
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.code.slug() == "type-unsaturated-application"),
        "{:?}",
        typed
            .defs
            .get(&typecheck::DefId(Intern::from_ref("value")))
            .map(|bind| &bind.return_type)
    );
}

#[test]
fn unused_generic_parameter_suggests_phantom_identity() {
    let typed = transformed("Count(unit) is in 0...255\n");
    let flaws = typed.all_flaws();
    let diagnostic = flaws
        .iter()
        .find_map(|(_, diagnostic)| {
            (diagnostic.code.slug() == "type-unused-generic-parameter").then_some(diagnostic)
        })
        .expect("unused generic flaw");

    assert!(
        diagnostic
            .help
            .as_deref()
            .is_some_and(|help| help.contains("#phantom(unit)"))
    );
}

#[test]
fn phantom_attribute_must_name_a_declared_parameter() {
    let typed = transformed("#phantom(other)\nCount(unit) is in 0...255\n");

    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| { diagnostic.code.slug() == "type-unknown-phantom-parameter" })
    );
}

fn target_unit_source(extra: &str) -> String {
    format!(
        "index_bits := #index_bits(@())\nindex_sign := 1 << (index_bits - 1)\nmax_count := index_sign - 1\nmin_offset := 0 - max_count\nmax_alignment := 1 << (index_bits - 2)\n#phantom(unit)\n#bits(index_bits)\nCount(unit) is in 0...max_count\n#phantom(unit)\n#bits(index_bits)\nOffset(unit) is in min_offset...max_count\n#bits(index_bits)\nAlignment is in 1...max_alignment and max_alignment % self = 0\n{extra}"
    )
}

fn target_unit_flaws(extra: &str) -> Vec<diagnostic::Diagnostic> {
    let source = target_unit_source(extra);
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux").expect("target"),
    );
    let mut asts = vec![parsed.ast];
    let preparation_flaws = typecheck::prepare_package_asts(&mut asts, &target);
    assert!(
        preparation_flaws.iter().all(Vec::is_empty),
        "{preparation_flaws:?}"
    );
    let package = transform_package_with_shared_context(
        asts,
        PackageTransformOptions::FULL.with_compile_target(target),
    );
    package
        .typed_asts
        .iter()
        .flat_map(typecheck::TypedFileAst::all_flaws)
        .map(|(_, diagnostic)| diagnostic.clone())
        .collect()
}

fn target_unit_flaw_codes(extra: &str) -> Vec<String> {
    target_unit_flaws(extra)
        .into_iter()
        .map(|diagnostic| diagnostic.code.slug().to_string())
        .collect()
}

const ALIGNMENT_DEFINITIONS: &str = "is_alignment(ref bytes is in 0...max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:\n    if bytes is in 1...max_alignment\n        candidate is in 1...max_alignment and max_alignment % candidate = 0: 1\n        while candidate < bytes is True\n            candidate *: 2\n        loop\n        if candidate = bytes is True\n            True\n        return\n        False\n    return\nreturn False\n\nAlignment has\n    new(bytes is in 1...max_alignment and max_alignment % bytes = 0) Self:\n    return bytes as Alignment\n\n    try_new(bytes is in 0...max_count):\n        if is_alignment(ref bytes) is True\n            Valid(bytes as Alignment)\n        return\n    return Invalid\n";

const LAYOUT_DEFINITIONS: &str = "Layout(element) has\n    count Count(element) and * #size(element) <= max_count\n    alignment Alignment and >= #alignment(element)\n\n    new(count Count(element) and * #size(element) <= max_count) Self:\n    return Self(\n        count: count\n        alignment: Alignment.new(#alignment(element))\n    )\n\n    new(\n        count Count(element) and * #size(element) <= max_count\n        alignment Alignment and >= #alignment(element)\n    ) Self:\n    return Self(count: count, alignment: alignment)\n\n    try_new(count is in 0...max_count):\n    return Self.try_new(count: count, alignment: #alignment(element))\n\n    try_new(count is in 0...max_count, alignment is in 0...max_count):\n        if is_alignment(ref alignment) is True and alignment >= #alignment(element) is True and count * #size(element) <= max_count is True\n            Valid(Self(\n                count: count as Count(element)\n                alignment: alignment as Alignment\n            ))\n        return\n    return Invalid\n";

#[test]
fn targetless_unit_declarations_remain_symbolic_without_host_fallback() {
    let parsed = target_unit_source("").parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let package =
        transform_package_with_shared_context(vec![parsed.ast], PackageTransformOptions::IDE);
    let flaws = package
        .typed_asts
        .iter()
        .flat_map(typecheck::TypedFileAst::all_flaws)
        .collect::<Vec<_>>();

    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn target_shaped_unit_declarations_resolve_for_concrete_target() {
    let flaws = target_unit_flaw_codes("");

    assert!(flaws.is_empty(), "{flaws:?}");
}

#[test]
fn alignment_accepts_power_of_two_and_rejects_other_finite_candidates() {
    let accepted = target_unit_flaw_codes("value Alignment: 8 as Alignment\n");
    assert!(accepted.is_empty(), "{accepted:?}");

    let rejected = target_unit_flaw_codes("value Alignment: 3 as Alignment\n");

    assert!(
        rejected
            .iter()
            .any(|code| code == "type-integer-narrowing-failed"),
        "{rejected:?}"
    );
}

#[test]
fn is_alignment_loop_preserves_the_power_of_two_invariant() {
    let flaws = target_unit_flaws(ALIGNMENT_DEFINITIONS);

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn layout_constructors_preserve_extent_and_alignment_refinements() {
    let source = target_unit_source(&format!("{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}"));
    let parsed = source.parse_source_full();
    assert!(
        parsed
            .ast
            .defs
            .contains_key(&Intern::from_ref("Layout.try_new$arity1")),
        "{:?}",
        parsed.ast.defs.keys().collect::<Vec<_>>()
    );
    assert!(
        parsed
            .ast
            .defs
            .contains_key(&Intern::from_ref("Layout.try_new$arity2")),
        "{:?}",
        parsed.ast.defs.keys().collect::<Vec<_>>()
    );
    let flaws = target_unit_flaws(&format!("{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}"));

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn layout_accepts_natural_and_custom_overalignment() {
    let uses = "Word has value Alignment\nnatural() Layout(Word): Layout.new(1 as Count(Word))\nover() Layout(Word): Layout.new(1 as Count(Word), 16 as Alignment)\n";
    let flaws = target_unit_flaws(&format!(
        "{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}\n{uses}"
    ));

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn layout_rejects_static_underalignment() {
    let uses = "Word has value Alignment\nunder() Layout(Word): Layout.new(1 as Count(Word), 1 as Alignment)\n";
    let flaws = target_unit_flaw_codes(&format!(
        "{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}\n{uses}"
    ));

    assert!(
        flaws
            .iter()
            .any(|code| code == "type-parameter-refinement-failed"),
        "{flaws:#?}"
    );
}

#[test]
fn layout_rejects_static_oversized_extent() {
    let uses = "Word has value Alignment\ntoo_large() Layout(Word): Layout.new(max_count as Count(Word))\n";
    let flaws = target_unit_flaw_codes(&format!(
        "{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}\n{uses}"
    ));

    assert!(
        flaws
            .iter()
            .any(|code| code == "type-parameter-refinement-failed"),
        "{flaws:#?}"
    );
}

#[test]
fn nullary_failure_cannot_discard_a_nominal_constructor_input() {
    let typed = transformed(
        "Int is in 0...255\nCandidate has value Pointer(Int)\nconstruct(candidate Candidate):\n    if 1 = 1 is True\n        Valid(candidate)\n    return\n    return Invalid\n",
    );
    let flaws = typed.all_flaws();

    assert!(
        flaws
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-owned-param-not-consumed"),
        "{flaws:#?}"
    );
}

#[test]
fn failure_payload_discharges_a_nominal_constructor_input() {
    let typed = transformed(
        "Int is in 0...255\nCandidate has value Pointer(Int)\nconstruct(candidate Candidate):\n    if 1 = 1 is True\n        Valid(candidate)\n    return\n    return Invalid(candidate)\n",
    );
    let flaws = typed.all_flaws();

    assert!(
        flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() != "type-owned-param-not-consumed"),
        "{flaws:#?}"
    );
}
