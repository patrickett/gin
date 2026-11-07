use ast::{Import, ImportSource, Item, SourceFile, SpanId};

fn imports(file: &SourceFile) -> Vec<&Import> {
    file.items
        .iter()
        .filter_map(|item| match &item.value {
            Item::Import(import) => Some(import.as_ref()),
            _ => None,
        })
        .collect()
}

fn spelling(file: &SourceFile, span: SpanId) -> &str {
    file.span_table.get(span).extract(&file.text)
}

#[test]
fn real_fixture_imports_preserve_sources_members_and_aliases() {
    // From utf8_differential_records.gin and conditional_imports/main.gin.
    let source = concat!(
        "use core.string.utf8 validate_utf8, Utf8Validation, Utf8Error\n",
        "use core.io.platform as platform\n",
        "use './build/flags.gin' as env\n",
        "value := 1\n",
    );
    let output = parser::parse(source).unwrap();
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let imports = imports(&output.file);
    assert_eq!(imports.len(), 3);
    let entry = &imports[0].entries[0].value;
    let ImportSource::Module(path) = &entry.source.value else {
        panic!("expected module path");
    };
    assert_eq!(
        path.iter()
            .map(|name| name.value.as_str())
            .collect::<Vec<_>>(),
        ["core", "string", "utf8"]
    );
    assert_eq!(entry.members.len(), 3);
    assert_eq!(entry.members[1].value.path[0].value, "Utf8Validation");
    assert_eq!(spelling(&output.file, path[2].span_id), "utf8");
    let alias = imports[1].entries[0].value.alias.as_ref().unwrap();
    assert_eq!(spelling(&output.file, alias.span_id), "platform");
    assert_eq!(imports[2].entries[0].value.source.value, ImportSource::Path);
    assert_eq!(
        spelling(&output.file, imports[2].entries[0].value.source.span_id),
        "'./build/flags.gin'"
    );
    assert_eq!(output.file.items.len(), 4);
}

#[test]
fn duplicates_and_current_exports_remain_in_written_order() {
    let output = parser::parse(
        "use Str as text, Str, Byte\nuse self.primitive Bool, Bool as Flag, folder.Name\n",
    )
    .unwrap();
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let imports = imports(&output.file);
    assert_eq!(imports[0].entries.len(), 3);
    let first = &imports[0].entries[0].value;
    assert!(matches!(&first.source.value, ImportSource::Current(name) if name.value == "Str"));
    assert_eq!(first.alias.as_ref().unwrap().value, "text");
    let members = &imports[1].entries[0].value.members;
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].value.path[0].value, "Bool");
    assert_eq!(members[1].value.path[0].value, "Bool");
    assert_ne!(members[0].span_id, members[1].span_id);
    assert_eq!(members[2].value.path.len(), 2);
}

#[test]
fn indented_members_stop_before_following_item() {
    let output = parser::parse(
        "use '../math'\n    add\n    sub as subtract\nuse math\n    add, sub\nvalue := 1\n",
    )
    .unwrap();
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let imports = imports(&output.file);
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].entries[0].value.members.len(), 2);
    assert_eq!(imports[1].entries[0].value.members.len(), 2);
    assert_eq!(output.file.items.len(), 3);
}

#[test]
fn malformed_imports_recover_at_following_declaration() {
    for malformed in [
        "use",
        "use math.",
        "use math as",
        "use math as Namespace",
        "use math as self",
        "use math.self",
        "use core folder.self",
        "use math add as one as whole",
        "use math add sub",
        "use math.(add, sub)",
        "use 'math'.add",
        "use math add,",
        "use math\n    add as",
    ] {
        let source = format!("{malformed}\nvalue := 1\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors(), "{source}");
        assert!(matches!(output.file.items[0].value, Item::Error));
        assert!(
            output.file.items.iter().any(|item| match &item.value {
                Item::Binding(binding) => binding.name.value == "value",
                _ => false,
            }),
            "{source}: {:?}",
            output.file.items
        );
    }
}

#[test]
fn quoted_utf8_path_spans_are_bytes_and_spelling_is_not_decoded() {
    let source = "-- π\nuse '../café/文.gin' as module\n";
    let output = parser::parse(source).unwrap();
    assert!(!output.has_errors(), "{:?}", output.diagnostics);
    let imports = imports(&output.file);
    let entry = &imports[0].entries[0].value;
    let path = output.file.span_table.get(entry.source.span_id);
    assert_eq!(path.extract(source), "'../café/文.gin'");
    assert_eq!(path.start(), source.find('\'').unwrap());
    assert!(source.is_char_boundary(path.end()));
    assert_eq!(
        spelling(&output.file, entry.alias.as_ref().unwrap().span_id),
        "module"
    );
}

#[test]
fn import_queries_locate_written_names_without_fabricating_declarations() {
    let source = "use core.string Text as Label, parse\nuse '../café' as module\nThing is Int\n";
    let file = parser::parse_strict(source).unwrap();
    for name in ["core", "string", "Text", "Label", "parse", "module"] {
        let hit = file.node_at_offset(source.find(name).unwrap()).unwrap();
        assert!(matches!(hit.node.kind, ast::SyntaxNodeKind::Name(found) if found == name));
        assert!(hit.declarations.is_empty());
        assert!(
            hit.ancestors
                .iter()
                .any(|node| matches!(node.kind, ast::SyntaxNodeKind::Import(_)))
        );
    }
    let path = file.node_at_offset(source.find("café").unwrap()).unwrap();
    assert!(matches!(
        path.node.kind, ast::SyntaxNodeKind::ImportSource(source)
            if matches!(source.value, ImportSource::Path)
    ));
    let outline: Vec<_> = file.declaration_outline().map(|entry| entry.name).collect();
    assert_eq!(outline, ["Thing"]);
}

#[test]
fn every_utf8_import_edit_prefix_remains_queryable() {
    let source = "use '../café/文'\n    Text as Label\n    parse\nThing is Int\n";
    for end in 0..=source.len() {
        if !source.is_char_boundary(end) {
            continue;
        }
        let prefix = &source[..end];
        let output = parser::parse(prefix).expect("bounded import edit");
        for offset in 0..=end {
            let hit = output.file.node_at_offset(offset);
            if !prefix.is_char_boundary(offset) {
                assert!(hit.is_none());
                continue;
            }
            let hit = hit.unwrap();
            assert!(prefix.get(hit.node.span.to_range()).is_some());
            for ancestor in hit.ancestors {
                assert!(ancestor.span.start() <= hit.node.span.start());
                assert!(hit.node.span.end() <= ancestor.span.end());
            }
        }
        for diagnostic in &output.diagnostics {
            assert!(prefix.get(diagnostic.span.to_range()).is_some());
        }
    }
}

#[test]
fn a_new_top_level_line_cannot_become_an_alias_of_a_closed_import_block() {
    for line in ["as alias", ", other"] {
        let source = format!("use math\n    add\n{line}\nnext := 2\n");
        let output = parser::parse(&source).unwrap();
        assert!(output.has_errors());
        assert_eq!(output.file.items.len(), 3);
        let import = imports(&output.file)[0];
        assert_eq!(import.entries.len(), 1);
        assert!(import.entries[0].value.alias.is_none());
        assert_eq!(
            spelling(&output.file, output.file.items[0].span_id),
            "use math\n    add"
        );
        assert_eq!(output.file.items[1].value, Item::Error);
        let next = output
            .file
            .node_at_offset(source.find("next").unwrap())
            .unwrap();
        assert!(matches!(
            next.node.kind,
            ast::SyntaxNodeKind::Name("next")
        ));
    }
}
