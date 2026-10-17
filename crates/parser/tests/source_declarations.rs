use diagnostic::Category;
use parser::query::SourceParseExt;
use parser::{SourceDeclaration, source_declarations};

fn checked_declarations(source: &str) -> Vec<SourceDeclaration> {
    let parsed = source.parse_source_full();
    assert!(
        !parsed
            .symptoms
            .iter()
            .any(|diagnostic| diagnostic.category == Category::Flaw),
        "{:?}",
        parsed.symptoms
    );
    let declarations = source_declarations(source);
    for declaration in &declarations {
        assert!(
            source.get(declaration.range.clone()).is_some(),
            "{declaration:?}"
        );
        assert!(!declaration.names.is_empty(), "{declaration:?}");
    }
    for pair in declarations.windows(2) {
        assert!(pair[0].range.end <= pair[1].range.start, "{pair:?}");
    }
    declarations
}

#[test]
fn duplicate_bindings_and_type_extensions_retain_every_source_occurrence() {
    let source = concat!(
        "Word is in 0 to 255\n",
        "Word has\n    finish(self):\n        return\n",
        "value Word\n",
        "value Word := 7\n",
        "read(value Word) Word: return value\n",
        "read(value Other) Other: return value\n",
    );
    let declarations = checked_declarations(source);
    let owners: Vec<_> = declarations
        .iter()
        .map(|declaration| declaration.owner.as_deref())
        .collect();
    assert_eq!(
        owners,
        [
            Some("Word"),
            Some("Word"),
            Some("value"),
            Some("value"),
            Some("read"),
            Some("read")
        ]
    );
    let pieces: Vec<_> = declarations
        .iter()
        .map(|declaration| &source[declaration.range.clone()])
        .collect();
    assert_eq!(pieces.concat(), source);
    assert_eq!(declarations[1].names, ["Word", "Word.finish"]);
}

#[test]
fn docs_attributes_and_inline_comments_follow_their_own_declaration() {
    let source = concat!(
        "-- ordinary leading comment stays\n\n",
        "--- café 🦀\n#inline\nfirst: 1 -- trailing comment\n",
        "\n--- Second declaration.\nsecond: 2\n",
        "\n-- ordinary trailing comment stays\n",
    );
    let declarations = checked_declarations(source);
    assert_eq!(declarations.len(), 2);
    assert_eq!(
        &source[declarations[0].range.clone()],
        "--- café 🦀\n#inline\nfirst: 1 -- trailing comment\n"
    );
    assert_eq!(
        &source[declarations[1].range.clone()],
        "--- Second declaration.\nsecond: 2\n"
    );
    let retained = format!(
        "{}{}",
        &source[..declarations[0].range.start],
        &source[declarations[0].range.end..]
    );
    assert!(retained.contains("-- ordinary leading comment stays"));
    assert!(retained.contains("--- Second declaration."));
    assert!(retained.contains("-- ordinary trailing comment stays"));
    assert!(!retained.contains("trailing comment\n\n--- Second"));
}

#[test]
fn a_block_does_not_consume_following_declaration_metadata() {
    let source = concat!(
        "first(value Word) Word:\n    return value\n",
        "\n--- Next declaration.\n#inline\nsecond: 2\n",
    );
    let declarations = checked_declarations(source);
    assert_eq!(declarations.len(), 2);
    assert_eq!(
        &source[declarations[0].range.clone()],
        "first(value Word) Word:\n    return value\n"
    );
    assert_eq!(
        &source[declarations[1].range.clone()],
        "--- Next declaration.\n#inline\nsecond: 2\n"
    );
}

#[test]
fn imports_have_bound_names_and_private_markers_remain_outside_ranges() {
    let source = concat!(
        "--| Module documentation stays.\n",
        "use 'values' first as one, second\n",
        "use 'other' as other\n",
        "public_value: 1\n",
        "private\n",
        "--- Private value.\nsecret: 2\n",
    );
    let declarations = checked_declarations(source);
    assert_eq!(declarations.len(), 4);
    assert_eq!(declarations[0].owner, None);
    assert_eq!(declarations[0].names, ["one", "second"]);
    assert_eq!(declarations[1].owner, None);
    assert_eq!(declarations[1].names, ["other"]);
    assert_eq!(
        &source[declarations[3].range.clone()],
        "--- Private value.\nsecret: 2\n"
    );
    assert!(source[..declarations[3].range.start].ends_with("private\n"));

    let source = "private secret: 2\n";
    let declarations = checked_declarations(source);
    assert_eq!(&source[declarations[0].range.clone()], "secret: 2");
    let retained = format!(
        "{}{}",
        &source[..declarations[0].range.start],
        &source[declarations[0].range.end..]
    );
    assert_eq!(retained, "private \n");
}

#[test]
fn method_owners_and_embedded_methods_use_canonical_definition_names() {
    let source = concat!(
        "Box has\n",
        "    read(self ref) Word: return 1\n",
        "    finish(self): return\n",
        "Box.write(self mut, value Word): return\n",
    );
    let declarations = checked_declarations(source);
    let parsed = source.parse_source_full();
    assert_eq!(declarations[0].owner.as_deref(), Some("Box"));
    assert_eq!(declarations[0].names, ["Box", "Box.finish", "Box.read"]);
    assert_eq!(declarations[1].owner.as_deref(), Some("Box.write"));
    for name in ["Box.finish", "Box.read", "Box.write"] {
        assert!(
            parsed.ast.defs.keys().any(|id| id.0.as_str() == name),
            "{name}"
        );
    }
}

#[test]
fn a_final_declaration_without_a_newline_retains_exact_bytes() {
    let source = "first: 1\nsecond: 'café 🦀' -- last comment";
    let declarations = checked_declarations(source);
    assert_eq!(
        &source[declarations[1].range.clone()],
        "second: 'café 🦀' -- last comment"
    );
    assert_eq!(declarations[1].range.end, source.len());
}
