use parser::query::SourceParseExt;

#[test]
fn parse_local_path_bundle_import() {
    let output = "use 'utils' Foo, Bar\n".parse_source_full();
    let imports: Vec<_> = output.ast.uses.to_vec();
    assert_eq!(imports.len(), 1);
    let import = &imports[0].0;
    assert_eq!(import.len(), 1);
    let mi = &import[0];
    match &mi.source {
        ast::ImportSource::LocalBundle(lb) => {
            assert_eq!(lb.local_path, Some(std::path::PathBuf::from("utils")));
            assert_eq!(lb.members.len(), 2);
            assert_eq!(lb.members[0].export.as_str(), "Foo");
            assert_eq!(lb.members[1].export.as_str(), "Bar");
        }
        other => panic!("expected LocalBundle, got {:?}", other),
    }
}

#[test]
fn parse_local_path_bundle_with_alias() {
    let output = "use 'utils' Foo, Bar as Baz\n".parse_source_full();
    let imports: Vec<_> = output.ast.uses.to_vec();
    let mi = &imports[0].0[0];
    match &mi.source {
        ast::ImportSource::LocalBundle(lb) => {
            assert_eq!(lb.local_path, Some(std::path::PathBuf::from("utils")));
            assert_eq!(lb.members.len(), 2);
            assert_eq!(lb.members[0].export.as_str(), "Foo");
            assert!(lb.members[0].alias.is_none());
            assert_eq!(lb.members[1].export.as_str(), "Bar");
            assert_eq!(lb.members[1].alias.unwrap().as_str(), "Baz");
        }
        other => panic!("expected LocalBundle, got {:?}", other),
    }
}

#[test]
fn parse_local_member_import() {
    let output = "use 'utils/math' add\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    let ast::ImportSource::LocalBundle(b) = &mi.source else {
        panic!("expected selected exports");
    };
    assert_eq!(
        b.local_path.as_deref(),
        Some(std::path::Path::new("utils/math"))
    );
    assert_eq!(b.members[0].export.as_str(), "add");
}

#[test]
fn rejects_gin_file_import_path() {
    let output = "use './a.gin'\n".parse_source_full();
    assert!(output.ast.uses.is_empty());
    assert!(
        output
            .symptoms
            .iter()
            .any(|d| d.message.contains("require an alias"))
    );
}

#[test]
fn parses_namespaced_gin_file_import() {
    let output = "use './allocator.gin' as allocator\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    assert_eq!(mi.alias.unwrap().as_str(), "allocator");
    match &mi.source {
        ast::ImportSource::LocalFile(path, _) => {
            assert_eq!(path.to_str().unwrap(), "./allocator.gin")
        }
        other => panic!("expected LocalFile, got {:?}", other),
    }
}

#[test]
fn rejects_gin_file_member_import() {
    let output = "use './allocator.gin' Count\n".parse_source_full();
    assert!(output.ast.uses.is_empty() || !output.symptoms.is_empty());
}

#[test]
fn parse_package_member_import() {
    let output = "use core Int\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    let ast::ImportSource::LocalBundle(b) = &mi.source else {
        panic!("expected selected exports");
    };
    assert_eq!(b.root.as_str(), "core");
    assert_eq!(b.members[0].export.as_str(), "Int");
}

#[test]
fn parse_dependency_bundle_with_folder_path() {
    let output = "use self.primitive Bool, List\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    match &mi.source {
        ast::ImportSource::LocalBundle(lb) => {
            assert_eq!(lb.root.as_str(), "self");
            assert_eq!(lb.path_segments.len(), 1);
            assert_eq!(lb.path_segments[0].as_str(), "primitive");
            assert_eq!(lb.members.len(), 2);
            assert_eq!(lb.members[0].export.as_str(), "Bool");
            assert_eq!(lb.members[1].export.as_str(), "List");
        }
        other => panic!("expected LocalBundle, got {:?}", other),
    }
}

#[test]
fn parse_dependency_bundle_dotted_members() {
    let output = "use core default.Default, arch.Architecture\n".parse_source_full();
    assert!(
        output.symptoms.is_empty(),
        "parse errors: {:?}",
        output.symptoms
    );
    let mi = &output.ast.uses[0].0[0];
    match &mi.source {
        ast::ImportSource::LocalBundle(lb) => {
            assert_eq!(lb.root.as_str(), "core");
            assert_eq!(lb.members.len(), 2);
            assert_eq!(lb.members[0].export.as_str(), "default.Default");
            assert_eq!(lb.members[1].export.as_str(), "arch.Architecture");
        }
        other => panic!("expected LocalBundle, got {:?}", other),
    }
}

#[test]
fn parse_dependency_bundle_import() {
    let output = "use core Int, Byte\n".parse_source_full();
    let imports: Vec<_> = output.ast.uses.to_vec();
    let mi = &imports[0].0[0];
    match &mi.source {
        ast::ImportSource::LocalBundle(lb) => {
            assert_eq!(lb.root.as_str(), "core");
            assert!(lb.local_path.is_none());
            assert_eq!(lb.members.len(), 2);
        }
        other => panic!("expected LocalBundle, got {:?}", other),
    }
}

#[test]
fn parse_current_module_tag_import() {
    let output = "use Str, Int, Byte\n".parse_source_full();
    let imports: Vec<_> = output.ast.uses.to_vec();
    assert_eq!(imports.len(), 1);
    let import = &imports[0].0;
    assert_eq!(import.len(), 3);

    // First: Str
    match &import[0].source {
        ast::ImportSource::CurrentModule { member } => {
            assert_eq!(member.export.as_str(), "Str");
            assert!(member.alias.is_none());
        }
        other => panic!("expected CurrentModule, got {:?}", other),
    }

    // Second: Int
    match &import[1].source {
        ast::ImportSource::CurrentModule { member } => {
            assert_eq!(member.export.as_str(), "Int");
        }
        other => panic!("expected CurrentModule, got {:?}", other),
    }

    // Third: Byte
    match &import[2].source {
        ast::ImportSource::CurrentModule { member } => {
            assert_eq!(member.export.as_str(), "Byte");
        }
        other => panic!("expected CurrentModule, got {:?}", other),
    }
}

#[test]
fn parse_current_module_tag_import_with_alias() {
    let output = "use Str as string, Int, Byte\n".parse_source_full();
    let imports: Vec<_> = output.ast.uses.to_vec();
    let mi = &imports[0].0[0];
    match &mi.source {
        ast::ImportSource::CurrentModule { member } => {
            assert_eq!(member.export.as_str(), "Str");
            assert_eq!(member.alias.unwrap().as_str(), "string");
        }
        other => panic!("expected CurrentModule, got {:?}", other),
    }
}

#[test]
fn source_first_imports_share_member_semantics() {
    for src in [
        "use '../math' add, sub as subtract\n",
        "use '../math'\n    add\n    sub as subtract\n",
        "use math add, sub as subtract\n",
        "use math\n    add\n    sub as subtract\n",
        "use math.numbers add, sub as subtract\n",
    ] {
        let out = format!("{src}\nvalue := 1\n").parse_source_full();
        assert!(out.symptoms.is_empty(), "{src}: {:?}", out.symptoms);
        assert_eq!(out.ast.uses.len(), 1);
        assert_eq!(out.ast.defs.len(), 1);
        let ast::ImportSource::LocalBundle(b) = &out.ast.uses[0].0[0].source else {
            panic!("expected member import: {src}");
        };
        assert_eq!(b.members.len(), 2);
        assert_eq!(b.members[0].export.as_str(), "add");
        assert_eq!(b.members[1].alias.unwrap().as_str(), "subtract");
    }
}

#[test]
fn source_first_import_stops_at_next_statement() {
    for src in [
        "use math add\nuse other sub\nvalue := 1\n",
        "use math\n    add\nuse other\n    sub\nvalue := 1\n",
        "use math\nvalue := 1\n",
    ] {
        let out = src.parse_source_full();
        assert!(out.symptoms.is_empty(), "{src}: {:?}", out.symptoms);
        assert_eq!(out.ast.defs.len(), 1);
    }
}

#[test]
fn source_first_import_requires_member_separators() {
    let out = "use math add sub\n".parse_source_full();
    assert!(
        out.symptoms
            .iter()
            .any(|d| d.code.slug() == "parse-expected-export-separator")
    );
}

#[test]
fn obsolete_import_selection_is_rejected() {
    for src in [
        "use math.(add, sub)",
        "use 'math'.add",
        "use 'math'.(add, sub)",
    ] {
        let out = src.parse_source_full();
        assert!(
            out.symptoms
                .iter()
                .any(|d| d.code.slug() == "parse-legacy-import-selection"),
            "{src}: {:?}",
            out.symptoms
        );
    }
}

#[test]
fn dotted_import_is_a_module_namespace() {
    let out = "use package.math\n".parse_source_full();
    assert!(out.symptoms.is_empty(), "{:?}", out.symptoms);
    let module = &out.ast.uses[0].0[0];
    assert!(matches!(module.source, ast::ImportSource::Package(_)));
    assert_eq!(module.effective_name(), "math");
}
