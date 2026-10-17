use diagnostic::Category;
use flask::CompileTarget;
use ginfmt::help;
use test_fixtures::TempPackage;

const GENERIC_HOLDER: &str = concat!(
    "Holder(t) has\n",
    "    storage t\n",
    "    release(self):\n",
    "        Holder(storage) := self\n",
    "        region: storage\n",
    "        region.release\n",
    "        return\n",
);

const TYPES: &str = "Word is in 0 to 255\nWord has\n    release(self):\n        return\nResource has\n    value Word\n    release(self): return self.value.release\nSession has\n    output Resource\n    original Resource\n    release(self):\n        self.output.release\n        self.original.release\n        return\n";

fn hints(source: &str) -> Vec<help::Help> {
    let package = TempPackage::new("redundant_destructure_bindings");
    package.write_flask("redundant_destructure_bindings");
    let path = package.write("main.gin", &format!("{TYPES}{source}"));
    help::suggestions(&path, CompileTarget::Library)
        .unwrap()
        .into_iter()
        .filter(|hint| hint.diagnostic.code.slug() == "fmt-redundant-destructure-binding")
        .collect()
}

#[test]
fn immediate_aliases_of_owned_destructured_fields_have_help() {
    let helps = hints(
        "run(session Session):\n    Session(output, original) := session\n    writer: output\n    saved: original\n    writer.release\n    saved.release\n    return\n",
    );
    assert_eq!(helps.len(), 2, "{helps:?}");
    for (hint, binding, source) in [
        (&helps[0], "writer", "output"),
        (&helps[1], "saved", "original"),
    ] {
        assert_eq!(hint.diagnostic.category, Category::Help);
        assert_eq!(hint.diagnostic.arg("binding"), Some(binding));
        assert_eq!(hint.diagnostic.arg("source"), Some(source));
        assert_eq!(hint.edits.len(), 2);
        assert!(
            hint.diagnostic
                .help
                .as_ref()
                .unwrap()
                .contains(&format!("{source}: {binding}"))
        );
    }
}

#[test]
fn member_bodies_and_control_flow_are_checked() {
    for body in [
        "Session has\n    close(self):\n        Session(output, original) := self\n        writer: output\n        writer.release\n        original.release\n        return\n",
        "run(session Session):\n    when True is True then\n        Session(output, original) := session\n        writer: output\n        writer.release\n        original.release\n        return\n    return\n",
    ] {
        assert_eq!(hints(body).len(), 1, "{body}");
    }
}

#[test]
fn borrowed_fields_typed_bindings_copies_and_later_bindings_are_not_alias_hints() {
    for source in [
        "run(session ref Session):\n    Session(output, original) := session\n    writer: output\n    return\n",
        "run(session Session):\n    Session(output, original) := session\n    writer Resource: output\n    writer.release\n    original.release\n    return\n",
        "run(session Session):\n    Session(output, original) := session\n    writer := output\n    writer.release\n    original.release\n    return\n",
        "run(session Session):\n    Session(output, original) := session\n    writer: output.copy\n    return\n",
        "run(session Session):\n    Session(output, original) := session\n    original.release\n    writer: output\n    writer.release\n    return\n",
        "run(session Session):\n    Session(output: writer, original: saved) := session\n    writer.release\n    saved.release\n    return\n",
    ] {
        assert!(hints(source).is_empty(), "{source}");
    }
}

#[test]
fn alias_fixes_combine_with_other_style_fixes_and_converge() {
    let package = TempPackage::new("redundant_destructure_fix");
    package.write_flask("redundant_destructure_fix");
    let source = format!(
        "{TYPES}value Word:\n    1\nrun(input Session):\n    Session(output, original) := input\n    writer: output\n    writer.release\n    original.release\n    return\n"
    );
    let path = package.write("main.gin", &source);
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 2);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        source
            .replace("value Word:\n    1", "value Word: 1")
            .replace(
                "Session(output, original) := input\n    writer: output",
                "Session(output: writer, original) := input"
            )
    );
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
}

fn fixed(source: &str) -> String {
    let package = TempPackage::new("redundant_destructure_fix_cases");
    package.write_flask("redundant_destructure_fix_cases");
    let path = package.write("main.gin", source);
    help::fix(&path, CompileTarget::Library).unwrap_or_else(|error| panic!("{error}\n{source:?}"));
    assert_eq!(help::fix(&path, CompileTarget::Library).unwrap(), 0);
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn unspecialized_generic_methods_offer_and_apply_alias_fixes() {
    let source = GENERIC_HOLDER;
    let helps = hints(source);
    assert_eq!(helps.len(), 1, "{helps:?}");
    assert_eq!(helps[0].edits.len(), 2);
    assert_eq!(
        fixed(source),
        source
            .replace("Holder(storage) := self", "Holder(storage: region) := self")
            .replace("        region: storage\n", "")
    );
}

#[test]
fn generic_method_extensions_also_offer_alias_fixes() {
    let source = GENERIC_HOLDER.replace("    release(self):", "Holder(t) has\n    release(self):");
    assert_eq!(hints(&source).len(), 1);
    assert_eq!(
        fixed(&source),
        source
            .replace("Holder(storage) := self", "Holder(storage: region) := self")
            .replace("        region: storage\n", "")
    );
}

#[test]
fn generic_borrowed_receivers_do_not_offer_alias_fixes() {
    for receiver in ["self ref", "self mut"] {
        let source = format!(
            "Holder(t) has\n    storage t\n    inspect({receiver}):\n        Holder(storage) := self\n        region: storage\n        return\n"
        );
        assert!(hints(&source).is_empty(), "{source}");
    }
}

#[test]
fn nested_borrowed_receivers_do_not_inherit_the_outer_ownership() {
    let source = GENERIC_HOLDER.replace(
        "        Holder(storage) := self",
        concat!(
            "        nested(self ref Holder(t)):\n",
            "            Holder(storage) := self\n",
            "            region: storage\n",
            "            return\n",
            "        Holder(storage) := self",
        ),
    );
    let helps = hints(&source);
    assert_eq!(helps.len(), 1, "{helps:?}");
    assert_eq!(
        fixed(&source),
        source.replace(
            "        Holder(storage) := self\n        region: storage\n        region.release",
            "        Holder(storage: region) := self\n        region.release",
        )
    );
}

#[test]
fn aliases_fold_into_explicit_patterns_and_fix_all_handles_multiple_fields() {
    let source = format!(
        "{TYPES}run(input Session):\n    Session(output: destination, original) := input\n    writer: destination\n    saved: original\n    writer.release\n    saved.release\n    return\n"
    );
    assert_eq!(
        fixed(&source),
        source
            .replace(
                "output: destination, original",
                "output: writer, original: saved"
            )
            .replace("    writer: destination\n", "")
            .replace("    saved: original\n", "")
    );
}

#[test]
fn alias_removal_preserves_comments_and_unicode() {
    let source = format!(
        "{TYPES}run(input Session):\n    -- before 🌍\n    Session(output, original) := input\n    writer: output -- keep 🌍\n    writer.release\n    original.release\n    return\n"
    );
    assert_eq!(
        fixed(&source),
        source
            .replace(
                "Session(output, original)",
                "Session(output: writer, original)"
            )
            .replace("writer: output -- keep", "-- keep")
    );
}

#[test]
fn reused_sources_and_colliding_pattern_names_have_no_automatic_fix() {
    for source in [
        "run(input Session):\n    Session(output, original) := input\n    writer: output\n    writer.release\n    original.release\n    return output\n",
        "run(input Session):\n    Session(output, original) := input\n    original: output\n    original.release\n    return\n",
    ] {
        let helps = hints(source);
        assert_eq!(helps.len(), 1, "{helps:?}");
        assert!(helps[0].edits.is_empty(), "{helps:?}");
    }
}

#[test]
fn nested_shadowing_and_strings_remain_unchanged() {
    let source = format!(
        "{TYPES}run(input Session):\n    Session(output, original) := input\n    writer: output\n    inner(writer Word) Word: return writer\n    text := \"writer output\"\n    writer.release\n    original.release\n    return\n"
    );
    assert_eq!(
        fixed(&source),
        source
            .replace(
                "Session(output, original)",
                "Session(output: writer, original)"
            )
            .replace("    writer: output\n", "")
    );
}
