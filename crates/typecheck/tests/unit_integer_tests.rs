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
    let typed = transformed(&format!(
        "{}CodePoint is in 0 to 1114111\nCodePoint has\n    finish(self):\n        return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\nHolder has\n    bytes Count(Byte), characters Count(CodePoint)\n    finish(self):\n        return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
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
    let typed = transformed(&format!(
        "{}#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\nByteCount is Count(Byte)\nHolder has\n    direct Count(Byte), alias ByteCount\n    finish(self):\n        return\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert_eq!(field_type(&typed, "direct"), field_type(&typed, "alias"));
}

#[test]
fn bare_generic_integer_application_is_rejected() {
    let typed = transformed(
        "#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\nvalue Count\n",
    );
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| diagnostic.code.slug() == "type-unsaturated-application"),
        "{:?}",
        typed
            .defs
            .get(&typecheck::DefId::named(Intern::from_ref("value")))
            .map(|bind| &bind.return_type)
    );
}

#[test]
fn unused_generic_parameter_suggests_phantom_identity() {
    let typed = transformed(
        "Count(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\n",
    );
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
    let typed = transformed(
        "#phantom(other)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\n",
    );

    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, diagnostic)| { diagnostic.code.slug() == "type-unknown-phantom-parameter" })
    );
}

fn target_unit_source(extra: &str) -> String {
    format!(
        "index_bits := #index_bits(@())\nindex_sign := 1 << (index_bits - 1)\nmax_count := index_sign - 1\nmin_offset := 0 - max_count\nmax_alignment := 1 << (index_bits - 2)\n#phantom(unit)\n#bits(index_bits)\nCount(unit) is in 0 to max_count\nCount(unit) has\n    finish(self):\n        return\n#phantom(unit)\n#bits(index_bits)\nOffset(unit) is in min_offset to max_count\nOffset(unit) has\n    finish(self):\n        return\n#bits(index_bits)\nAlignment is in 1 to max_alignment and max_alignment % self = 0\nAlignment has\n    finish(self):\n        return\n{extra}"
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
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

const ALIGNMENT_DEFINITIONS: &str = "is_alignment(bytes is in 0 to max_count) True thus bytes >= 1 and bytes <= max_alignment and max_alignment % bytes = 0 or False:\n    when bytes is in 1 to max_alignment then\n        candidate is in 1 to max_alignment and max_alignment % candidate = 0: 1\n        while candidate < bytes do\n            candidate *: 2\n        when candidate = bytes then\n            return True\n        return False\n    return False\n\nAlignment has\n    finish(self):\n        return\n    new(bytes is in 1 to max_alignment and max_alignment % bytes = 0) Self:\n        return bytes as Alignment\n\n    try_new(bytes is in 0 to max_count):\n        when is_alignment(bytes) is True then\n            return Valid(Alignment.new(1))\n        return Invalid\n";

const LAYOUT_DEFINITIONS: &str = "Layout(element) has\n    finish(self):\n        return\n    count Count(element) and * #size(element) <= max_count\n    alignment Alignment and >= #alignment(element)\n\n    new(c Count(element) and * #size(element) <= max_count) Self:\n        return Self(\n            count: c\n            alignment: Alignment.new(#alignment(element))\n        )\n\n    new(\n        c Count(element) and * #size(element) <= max_count\n        alignment Alignment and >= #alignment(element)\n    ) Self:\n        return Self(count: c, alignment: alignment)\n\n    try_new(c is in 0 to max_count):\n        return Self.try_new(c: c, alignment: #alignment(element))\n\n    try_new(c is in 0 to max_count, alignment is in 0 to max_count):\n        when alignment >= #alignment(element) and c * #size(element) <= max_count then\n            return Valid(Self(\n                    count: c as Count(element)\n                    alignment: Alignment.new(1)\n            ))\n        return Invalid\n";

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
        parsed.ast.defs.contains_key(&ast::DefinitionId::member(
            Intern::from_ref("Layout.try_new"),
            0
        )),
        "{:?}",
        parsed.ast.defs.keys().collect::<Vec<_>>()
    );
    assert!(
        parsed.ast.defs.contains_key(&ast::DefinitionId::member(
            Intern::from_ref("Layout.try_new"),
            1
        )),
        "{:?}",
        parsed.ast.defs.keys().collect::<Vec<_>>()
    );
    let flaws = target_unit_flaws(&format!("{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}"));

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn layout_accepts_natural_and_custom_overalignment() {
    let uses = "Word has\n    value Alignment\n    finish(self):\n        return\nnatural Layout(Word): Layout.new(1 as Count(Word))\nover Layout(Word): Layout.new(1 as Count(Word), 16 as Alignment)\n";
    let flaws = target_unit_flaws(&format!(
        "{ALIGNMENT_DEFINITIONS}\n{LAYOUT_DEFINITIONS}\n{uses}"
    ));

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn layout_rejects_static_underalignment() {
    let uses = "Word has\n    value Alignment\n    finish(self):\n        return\nunder Layout(Word): Layout.new(1 as Count(Word), 1 as Alignment)\n";
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
    let uses = "Word has\n    value Alignment\n    finish(self):\n        return\ntoo_large Layout(Word): Layout.new(max_count as Count(Word))\n";
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
        "Int is in 0 to 255\nInt.finish(self): return\nCandidate has\n    value @Int\n    finish(self): return\nconstruct(candidate own Candidate):\n    when 1 = 1 then\n        return Valid(candidate)\n    return Invalid\n",
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
        "Int is in 0 to 255\nInt.finish(self): return\nCandidate has\n    value @Int\n    finish(self): return\nconstruct(candidate Candidate):\n    when 1 = 1 then\n        return Valid(candidate)\n    return Invalid(candidate)\n",
    );
    let flaws = typed.all_flaws();

    assert!(flaws.is_empty(), "{flaws:#?}");
}

#[test]
fn qualified_target_sized_integer_bounds_follow_the_target() {
    let source = "width := #index_bits(@())\nsign := 1 << (width - 1)\nmaximum := sign - 1\nminimum := 0 - sign\n#phantom(unit)\n#bits(width)\nQuantity(unit) is in 0 to maximum\nQuantity(unit) has\n    finish(self):\n        return\n#phantom(unit)\n#bits(width)\nDisplacement(unit) is in minimum to maximum\nDisplacement(unit) has\n    finish(self):\n        return\n";
    for (triple, width) in [
        ("wasm32-unknown-unknown", 32),
        ("x86_64-unknown-linux-gnu", 64),
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut ast = parsed.ast.qualify_module_defs("sample.sizes");
        let target = flask::CompileTarget::Concrete(flask::TargetTriple::parse(triple).unwrap());
        let flaws = typecheck::prepare_parse_ast(&mut ast, &target);
        assert!(flaws.is_empty(), "{flaws:?}");
        let package = transform_package_with_shared_context(
            vec![ast],
            PackageTransformOptions::FULL.with_compile_target(target),
        );
        let typed = &package.typed_asts[0];
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        for name in ["Quantity", "Displacement"] {
            let ty = &typed.tags[&TagId(Intern::from_ref(name))].resolved_ty;
            assert_eq!(
                typecheck::representation::Repr::derive(ty, Some(&typed.type_registry)),
                Ok(typecheck::representation::Repr::Bits { width }),
            );
        }
    }
}

#[test]
fn qualified_constants_determine_record_array_capacity() {
    let source = "capacity := 4\nOctet is in 0 to 255\nOctet has\n    finish(self):\n        return\nPacket has\n    finish(self):\n        return\n    bytes Storage(Octet, capacity)\n#fixed_array(element, count)\nStorage(element, count BigInt) has\n    finish(self):\n        return\n";
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut ast = parsed.ast.qualify_module_defs("sample.storage");
    let target = flask::CompileTarget::Concrete(
        flask::TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap(),
    );
    assert!(typecheck::prepare_parse_ast(&mut ast, &target).is_empty());
    let package = transform_package_with_shared_context(
        vec![ast],
        PackageTransformOptions::FULL.with_compile_target(target),
    );
    let typed = &package.typed_asts[0];
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let ty = &typed.tags[&TagId(Intern::from_ref("Packet"))].resolved_ty;
    assert!(typecheck::representation::Repr::derive(ty, Some(&typed.type_registry)).is_ok());
    let ast::Ty::Record { fields, .. } = typed.type_registry.resolved_definition_for_type(ty)
    else {
        panic!("record")
    };
    assert_eq!(
        typecheck::representation::Repr::array_parts(&fields[0].1, Some(&typed.type_registry))
            .unwrap()
            .1,
        4
    );
}

#[test]
fn unsigned_target_sized_bounds_cover_the_index_domain() {
    let source = "width := #index_bits(@())\nmaximum := (1 << width) - 1\n#bits(width)\nExtent is in 0 to maximum\nExtent has\n    finish(self):\n        return\n";
    for (triple, width) in [
        ("wasm32-unknown-unknown", 32),
        ("x86_64-unknown-linux-gnu", 64),
        ("aarch64-unknown-linux-gnu", 64),
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut ast = parsed.ast.qualify_module_defs("sample.sizes");
        let target = flask::CompileTarget::Concrete(flask::TargetTriple::parse(triple).unwrap());
        assert!(typecheck::prepare_parse_ast(&mut ast, &target).is_empty());
        let package = transform_package_with_shared_context(
            vec![ast],
            PackageTransformOptions::FULL.with_compile_target(target),
        );
        let typed = &package.typed_asts[0];
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let ty = &typed.tags[&TagId(Intern::from_ref("Extent"))].resolved_ty;
        assert_eq!(typed.type_registry.integer_width_for_type(ty), Some(width));
        let domain = typed
            .type_registry
            .integer_validity_for_type(ty)
            .unwrap()
            .domain();
        let hull = domain.storage_hull().unwrap();
        assert_eq!(hull.min().to_string(), "0");
        assert_eq!(hull.max().to_string(), ((1u128 << width) - 1).to_string());
    }
}
