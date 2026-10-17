use parser::query::SourceParseExt;
use typecheck::FileId;
use typecheck::transform::transform_file;

#[test]
fn local_generic_annotations_preserve_explicit_type_arguments() {
    for (annotation, accepted) in [
        ("Count(Byte)", true),
        ("Count(unit: Byte)", true),
        ("Count(CodePoint)", false),
        ("Count(unit: CodePoint)", false),
    ] {
        let source = format!(
            "{}CodePoint is in 0 to 255\nCodePoint has\n    finish(self):\n        return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\ncheck(value Count(Byte)):\n    local {annotation}: value\n    local.finish\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{annotation}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if accepted {
            assert!(flaws.is_empty(), "{annotation}: {flaws:?}");
            let value = typed
                .places
                .iter()
                .find(|place| place.name.as_str() == "value")
                .expect("parameter place");
            let local = typed
                .places
                .iter()
                .find(|place| place.name.as_str() == "local")
                .expect("local place");
            assert_eq!(local.ty, value.ty, "{annotation}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-binding-mismatch"),
                "{annotation}: {flaws:?}"
            );
        }
    }
}

#[test]
fn local_generic_annotations_preserve_explicit_constant_arguments() {
    for (annotation, accepted) in [
        ("Packet(2)", true),
        ("Packet(count: 2)", true),
        ("Packet", true),
        ("Packet(3)", false),
        ("Packet(count: 3)", false),
    ] {
        let source = format!(
            "{}#phantom(count)\nPacket(count BigInt) has\n    marker Byte\n    finish(self):\n        return\ncheck(value Packet(2)):\n    local {annotation}: value\n    local.finish\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{annotation}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if accepted {
            assert!(flaws.is_empty(), "{annotation}: {flaws:?}");
            let value = typed
                .places
                .iter()
                .find(|place| place.name.as_str() == "value")
                .expect("parameter place");
            let local = typed
                .places
                .iter()
                .find(|place| place.name.as_str() == "local")
                .expect("local place");
            assert_eq!(local.ty, value.ty, "{annotation}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-binding-mismatch"),
                "{annotation}: {flaws:?}"
            );
        }
    }
}

#[test]
fn bare_generic_record_annotation_infers_initializer_arguments() {
    let parsed = format!("{}Box(element) has\n    value element\n    finish(self):\n        return\ncheck(value Box(Byte)):\n    local Box: value\n    local.finish\n    return\n", test_fixtures::preludes::BYTE_PRELUDE).parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let value = typed
        .places
        .iter()
        .find(|place| place.name.as_str() == "value")
        .expect("parameter place");
    let local = typed
        .places
        .iter()
        .find(|place| place.name.as_str() == "local")
        .expect("local place");
    assert_eq!(local.ty, value.ty);
}

#[test]
fn bare_local_annotations_keep_declared_generic_defaults() {
    for (initializer, accepted) in [("Byte", true), ("CodePoint", false)] {
        let source = format!(
            "{}CodePoint is in 0 to 255\nCodePoint has\n    finish(self):\n        return\nBox(element: Byte) has\n    value element\n    finish(self):\n        return\ncheck(value Box({initializer})):\n    local Box: value\n    local.finish\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{initializer}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if accepted {
            assert!(flaws.is_empty(), "{initializer}: {flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-binding-mismatch"),
                "{initializer}: {flaws:?}"
            );
        }
    }
}

#[test]
fn reference_annotations_keep_applied_generic_arguments() {
    for convention in ["ref", "mut", ""] {
        for (annotation, accepted) in [("Byte", true), ("CodePoint", false)] {
            let source = format!(
                "{}CodePoint is in 0 to 255\nCodePoint has\n    finish(self):\n        return\n#phantom(unit)\nCount(unit) is in 0 to 255\nCount(unit) has\n    finish(self):\n        return\ncheck(value {convention} Count(Byte)):\n    local ref Count({annotation}): value\n    return\n",
                test_fixtures::preludes::BYTE_PRELUDE
            );
            let parsed = source.parse_source_full();
            assert!(
                parsed.symptoms.is_empty(),
                "{annotation}: {:?}",
                parsed.symptoms
            );
            let typed = transform_file(parsed.ast, FileId(0));
            let flaws = typed.all_flaws();
            if accepted {
                assert!(flaws.is_empty(), "{annotation}: {flaws:?}");
            } else {
                assert!(
                    flaws
                        .iter()
                        .any(|(_, flaw)| flaw.code.slug() == "type-binding-mismatch"),
                    "{annotation}: {flaws:?}"
                );
            }
        }
    }
}

#[test]
fn bare_local_annotations_keep_declared_constant_defaults() {
    for (initializer, accepted) in [(2, true), (3, false)] {
        let source = format!(
            "{}#phantom(count)\nPacket(count: 2) has\n    marker Byte\n    finish(self):\n        return\ncheck(value Packet({initializer})):\n    local Packet: value\n    local.finish\n    return\n",
            test_fixtures::preludes::BYTE_PRELUDE
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{initializer}: {:?}",
            parsed.symptoms
        );
        let typed = transform_file(parsed.ast, FileId(0));
        let flaws = typed.all_flaws();
        if accepted {
            assert!(flaws.is_empty(), "{initializer}: {flaws:?}");
        } else {
            assert!(
                flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "type-binding-mismatch"),
                "{initializer}: {flaws:?}"
            );
        }
    }
}
