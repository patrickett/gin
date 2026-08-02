//! Navigation targets for import paths and public symbols.

use ast::{BundleExportImport, ImportSource, LocalBundleImport, ModPath, Spanned};
use diagnostic::SpanId;
use internment::Intern;
use parser::query::SourceParseExt;
use resolve::{
    CursorDefinition, ImportNavLocation, cursor_definition, import_source_nav_location,
    package_import_part_location, public_symbol_def_location,
};
use test_fixtures::TempPackage;
use test_fixtures::gin_core::{BOOL_GIN, COPY_GIN, SIZED_GIN, TYPE_GIN};

fn pkg_with_gin_core_fixtures() -> TempPackage {
    let pkg = TempPackage::new("gin_core_fixtures");
    pkg.write("marker/sized.gin", SIZED_GIN);
    pkg.write("reflect/type.gin", TYPE_GIN);
    pkg.write("marker/copy.gin", COPY_GIN);
    pkg.write("primitive/bool.gin", BOOL_GIN);
    pkg
}

#[test]
fn import_source_package_nested_folder_manifest() {
    let pkg = TempPackage::new("nested_folder");
    pkg.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"dep":{"path":"dep"}}}"#,
    );
    pkg.write(
        "dep/flask.jsonc",
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    pkg.write(
        "dep/foldermod/flask.jsonc",
        r#"{"name":"foldermod","version":"0.0.0","authors":[]}"#,
    );
    let main = pkg.write("main.gin", "main:\n    return 0\n");

    let mp = Spanned::new(
        ModPath::new(
            Intern::<String>::from_ref("dep"),
            vec![Intern::<String>::from_ref("foldermod")],
        ),
        SpanId::new(0),
    );

    let nav = import_source_nav_location(&main, &ImportSource::Package(mp), &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("nested folder manifest");
    assert_eq!(
        nav,
        ImportNavLocation::Manifest(pkg.join("dep/foldermod/flask.jsonc"))
    );
}

#[test]
fn package_part_dep_root_and_nested() {
    let pkg = TempPackage::new("dep_a_b");
    pkg.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"dep":{"path":"dep"}}}"#,
    );
    pkg.write(
        "dep/flask.jsonc",
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    pkg.write(
        "dep/a/flask.jsonc",
        r#"{"name":"dep_a","version":"0.0.0","authors":[]}"#,
    );
    pkg.write(
        "dep/a/b/flask.jsonc",
        r#"{"name":"dep_ab","version":"0.0.0","authors":[]}"#,
    );
    let main = pkg.write("main.gin", "main:\n    return 0\n");

    let seg_a = Intern::<String>::from_ref("a");
    let seg_b = Intern::<String>::from_ref("b");

    let root = package_import_part_location(&main, "dep", &[seg_a, seg_b], 0, &|p| {
        resolve::ParsedFile::read(p)
    })
    .unwrap();
    assert_eq!(
        root,
        ImportNavLocation::Manifest(pkg.join("dep/flask.jsonc"))
    );

    let mid = package_import_part_location(&main, "dep", &[seg_a, seg_b], 1, &|p| {
        resolve::ParsedFile::read(p)
    })
    .unwrap();
    assert_eq!(
        mid,
        ImportNavLocation::Manifest(pkg.join("dep/a/flask.jsonc"))
    );

    let deep = package_import_part_location(&main, "dep", &[seg_a, seg_b], 2, &|p| {
        resolve::ParsedFile::read(p)
    })
    .unwrap();
    assert_eq!(
        deep,
        ImportNavLocation::Manifest(pkg.join("dep/a/b/flask.jsonc"))
    );
}

#[test]
fn public_symbol_def_location_exact_span() {
    let pkg = TempPackage::new("core_bool");
    pkg.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
    );
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    pkg.write("core/bool.gin", BOOL_GIN);
    pkg.write("main.gin", "use core.true\n\nmain:\n    return 0\n");

    let loc =
        public_symbol_def_location(&pkg.join("core"), "true", &|p| resolve::ParsedFile::read(p))
            .expect("true def");

    assert_eq!(loc.file, pkg.join("core/bool.gin"));
    let source = std::fs::read_to_string(&loc.file).unwrap();
    let snippet = &source[loc.byte_range.start..loc.byte_range.end];
    assert!(
        snippet.contains("true"),
        "span should cover `true` bind, got {snippet:?}"
    );
}

#[test]
fn cursor_definition_record_in_sized_pattern_to_reflect_type() {
    let pkg = pkg_with_gin_core_fixtures();
    let sized = pkg.join("marker/sized.gin");
    let type_gin = pkg.join("reflect/type.gin");
    let src = std::fs::read_to_string(&sized).unwrap();
    let output = src.parse_source_full();
    let arm = "    Record(_, fields)   then sum_named(fields)";
    let arm_start = src.find(arm).expect("Record arm in sized.gin");
    let record_pos = arm_start + arm.find("Record").expect("Record in pattern");

    let def = cursor_definition(&sized, &output.ast, &src, record_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-def on Record in pattern");

    match def {
        CursorDefinition::OtherFile(loc) => {
            assert_eq!(loc.file, type_gin);
            let type_src = std::fs::read_to_string(&loc.file).unwrap();
            let snippet = &type_src[loc.byte_range.start..loc.byte_range.end];
            assert_eq!(
                snippet, "Record",
                "span should cover variant name in type.gin"
            );
        }
        other => panic!("expected OtherFile to type.gin, got {other:?}"),
    }
}

#[test]
fn cursor_definition_all_variants_copy_when_subject_to_param() {
    let pkg = pkg_with_gin_core_fixtures();
    let copy = pkg.join("marker/copy.gin");
    let src = std::fs::read_to_string(&copy).unwrap();
    let output = src.parse_source_full();
    let def = "all_variants_copy(variants List(VariantShape)) Bool := when variants is";
    let def_start = src.find(def).expect("all_variants_copy definition");
    let param_pos = def_start + def.find("variants List").expect("variants param");
    let subject_pos = def_start + def.rfind("variants").expect("variants subject");

    let def = cursor_definition(&copy, &output.ast, &src, subject_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-definition for variants subject");

    assert_eq!(
        def,
        CursorDefinition::SameFile(param_pos..param_pos + "variants".len())
    );
}

#[test]
fn cursor_definition_false_in_ref_pattern_to_bool_variant() {
    let pkg = pkg_with_gin_core_fixtures();
    let copy = pkg.join("marker/copy.gin");
    let bool_gin = pkg.join("primitive/bool.gin");
    let src = std::fs::read_to_string(&copy).unwrap();
    let output = src.parse_source_full();
    let arm = "    Ref(_, False)       then True";
    let arm_start = src.find(arm).expect("Ref arm in copy.gin");
    let false_pos = arm_start + arm.find("False").expect("False in pattern");

    let def = cursor_definition(&copy, &output.ast, &src, false_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-def on False in Ref pattern");

    match def {
        CursorDefinition::OtherFile(loc) => {
            assert_eq!(loc.file, bool_gin);
            let bool_src = std::fs::read_to_string(&loc.file).unwrap();
            let snippet = &bool_src[loc.byte_range.start..loc.byte_range.end];
            assert_eq!(
                snippet, "False",
                "span should cover variant name in bool.gin"
            );
        }
        other => panic!("expected OtherFile to bool.gin, got {other:?}"),
    }
}

#[test]
fn cursor_definition_false_in_then_body_to_bool_variant() {
    let pkg = pkg_with_gin_core_fixtures();
    let copy = pkg.join("marker/copy.gin");
    let bool_gin = pkg.join("primitive/bool.gin");
    let src = std::fs::read_to_string(&copy).unwrap();
    let output = src.parse_source_full();
    let arm = "    Ref(_, True)        then False";
    let arm_start = src.find(arm).expect("Ref/True arm in copy.gin");
    let false_pos = arm_start + arm.rfind("False").expect("False in then body");

    let def = cursor_definition(&copy, &output.ast, &src, false_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-def on False in then body");

    match def {
        CursorDefinition::OtherFile(loc) => {
            assert_eq!(loc.file, bool_gin);
            let bool_src = std::fs::read_to_string(&loc.file).unwrap();
            let snippet = &bool_src[loc.byte_range.start..loc.byte_range.end];
            assert_eq!(snippet, "False");
        }
        other => panic!("expected OtherFile to bool.gin, got {other:?}"),
    }
}

#[test]
fn cursor_definition_body_import() {
    let pkg = TempPackage::new("body_import");
    pkg.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"core":{"path":"core"}}}"#,
    );
    pkg.write(
        "core/flask.jsonc",
        r#"{"name":"core","version":"0.0.0","authors":[]}"#,
    );
    pkg.write(
        "core/bool.gin",
        "Bool is True or False\n\ntrue  := Bool.True\n",
    );
    let main = pkg.write("main.gin", "use core.true\n\nmain:\n    true\nreturn\n");

    let source = std::fs::read_to_string(&main).unwrap();
    let output = source.parse_source_full();
    let byte = source
        .find("main:\n    true")
        .map(|i| i + "main:\n    ".len())
        .expect("`true` in main body");

    let def = cursor_definition(&main, &output.ast, &source, byte, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto on imported true in body");

    match def {
        CursorDefinition::OtherFile(loc) => {
            assert_eq!(loc.file, pkg.join("core/bool.gin"));
        }
        other => panic!("expected OtherFile for body reference, got {other:?}"),
    }
}

#[test]
fn cursor_definition_dep_bundle_member_in_folder() {
    let pkg = TempPackage::new("dep_bundle_goto");
    pkg.write_flask_with_deps("core", r#"{"self":{"path":"."}}"#);
    pkg.write("primitive/bool.gin", "Bool is Unit\n");
    pkg.write(
        "marker/copy.gin",
        "use self.primitive.(Bool)\n\nCopy has \n",
    );

    let copy = pkg.join("marker/copy.gin");
    let source = std::fs::read_to_string(&copy).unwrap();
    let output = source.parse_source_full();
    let bool_pos = source.find("Bool").expect("Bool in use line");

    let def = cursor_definition(&copy, &output.ast, &source, bool_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-def on Bool");

    match def {
        CursorDefinition::Import {
            target: ImportNavLocation::Definition(loc),
            ..
        } => {
            assert_eq!(loc.file, pkg.join("primitive/bool.gin"));
            assert!(loc.byte_range.start < loc.byte_range.end);
        }
        other => panic!("expected Definition in bool.gin, got {other:?}"),
    }
}

#[test]
fn cursor_definition_body_bundle_flat_name() {
    let pkg = TempPackage::new("body_bundle_goto");
    pkg.write_flask_with_deps("core", r#"{"core":{"path":"."}}"#);
    pkg.write("default/default.gin", "Default(value) has default value\n");
    pkg.write("target/arch/arch.gin", "Architecture is 'x86_64'\n");
    const TARGET: &str = "\
use core.(default.Default, target.arch.Architecture)

Target has arch Architecture
";
    let target_path = pkg.write("target/target.gin", TARGET);
    let source = std::fs::read_to_string(&target_path).unwrap();
    let output = source.parse_source_full();
    let body_start = source.find("Target has").expect("Target declare");
    let arch_pos = source[body_start..]
        .find("Architecture")
        .map(|i| body_start + i)
        .expect("Architecture in Target has ...");

    let def = cursor_definition(&target_path, &output.ast, &source, arch_pos, &|p| {
        resolve::ParsedFile::read(p)
    })
    .expect("goto-def on Architecture in Target has ...");

    match def {
        CursorDefinition::OtherFile(loc) => {
            assert_eq!(loc.file, pkg.join("target/arch/arch.gin"));
        }
        CursorDefinition::Import {
            target: ImportNavLocation::Definition(loc),
            ..
        } => {
            assert_eq!(loc.file, pkg.join("target/arch/arch.gin"));
        }
        other => panic!("expected definition in arch.gin, got {other:?}"),
    }
}

#[test]
fn import_source_dep_bundle_manifest() {
    let pkg = TempPackage::new("dep_bundle");
    pkg.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"utils":{"path":"utils"}}}"#,
    );
    pkg.write("main.gin", "use utils.(io)\n\nmain:\n    return 0\n");
    pkg.write(
        "utils/flask.jsonc",
        r#"{"name":"utils","version":"0.0.0","authors":[]}"#,
    );

    let main = pkg.join("main.gin");
    let lb = LocalBundleImport {
        root: Intern::<String>::from_ref("utils"),
        path_segments: Vec::new(),
        members: vec![BundleExportImport {
            export: Intern::<String>::from_ref("io"),
            alias: None,
            span: SpanId::new(0),
        }],
        span: SpanId::new(0),
        local_path: None,
    };

    let nav = import_source_nav_location(&main, &ImportSource::LocalBundle(lb), &|p| {
        resolve::ParsedFile::read(p)
    })
    .unwrap();
    assert_eq!(
        nav,
        ImportNavLocation::Manifest(pkg.join("utils/flask.jsonc"))
    );
}
