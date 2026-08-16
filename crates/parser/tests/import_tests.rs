use parser::query::SourceParseExt;

#[test]
fn parse_local_path_bundle_import() {
    let output = "use 'utils'.(Foo, Bar)\n".parse_source_full();
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
    let output = "use 'utils'.(Foo, Bar as Baz)\n".parse_source_full();
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
    let output = "use 'utils/math'.add\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    match &mi.source {
        ast::ImportSource::LocalMember(m) => {
            assert_eq!(m.local_path, Some(std::path::PathBuf::from("utils/math")));
            assert_eq!(m.member.export.as_str(), "add");
        }
        other => panic!("expected LocalMember, got {:?}", other),
    }
}

#[test]
fn rejects_gin_file_import_path() {
    let output = "use './a.gin'\n".parse_source_full();
    assert!(output.ast.uses.is_empty() || !output.symptoms.is_empty());
}

#[test]
fn parse_package_member_import() {
    let output = "use core.Int\n".parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    match &mi.source {
        ast::ImportSource::Package(mp) => {
            assert_eq!(mp.root.as_str(), "core");
            assert_eq!(mp.segments.len(), 1);
            assert_eq!(mp.segments[0].as_str(), "Int");
        }
        other => panic!("expected Package member import, got {:?}", other),
    }
}

#[test]
fn parse_prefer_member_import_diagnostic() {
    let output = "use core.(Int)\n".parse_source_full();
    assert!(output.symptoms.iter().any(|d| {
        d.code.slug() == "use-prefer-member-import"
            && d.message == "prefer use core.Int over a single-item .(...) bundle"
    }));
}

#[test]
fn parse_dependency_bundle_with_folder_path() {
    let output = "use self.primitive.(Bool, List)\n".parse_source_full();
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
    let output = "use core.(default.Default, arch.Architecture)\n".parse_source_full();
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
    let output = "use core.(Int, Byte)\n".parse_source_full();
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
