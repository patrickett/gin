use parser::query::SourceParseExt;
use resolve::{
    CursorDefinition, ImportNavLocation, ParsedFile, cursor_definition, hover_for_import_target,
    resolve_import_at,
};
use test_fixtures::TempPackage;

#[test]
fn dependency_symbol_hover_respects_folder_module_path() {
    let package = TempPackage::new("dependency_import_hover");
    package.write(
        "flask.jsonc",
        r#"{"name":"root","version":"0.0.0","authors":[],"dependencies":{"dep":{"path":"dep"}}}"#,
    );
    package.write(
        "dep/flask.jsonc",
        r#"{"name":"dep","version":"0.0.0","authors":[]}"#,
    );
    package.write("dep/string/string.gin", "ToString has to_string String\n");
    let source = "use dep.ToString\nuse dep.string.ToString\n";
    let main = package.write("main.gin", source);
    let parsed = source.parse_source_full();

    let root_byte = source.find("ToString").unwrap() + 1;
    let root_target = resolve_import_at(&parsed.ast, source, root_byte).unwrap();
    assert!(hover_for_import_target(&main, root_target, &|path| ParsedFile::read(path)).is_none());

    let nested_byte = source.rfind("ToString").unwrap() + 1;
    let nested_target = resolve_import_at(&parsed.ast, source, nested_byte).unwrap();
    let hover = hover_for_import_target(&main, nested_target, &|path| ParsedFile::read(path))
        .expect("nested dependency symbol hover");
    assert!(hover.contains("ToString has to_string String"));

    assert!(
        cursor_definition(&main, &parsed.ast, source, root_byte, &|path| {
            ParsedFile::read(path)
        })
        .is_none()
    );

    let definition = cursor_definition(&main, &parsed.ast, source, nested_byte, &|path| {
        ParsedFile::read(path)
    })
    .expect("nested dependency symbol definition");
    assert!(matches!(
        definition,
        CursorDefinition::Import {
            target: ImportNavLocation::Definition(location),
            ..
        } if location.file == package.join("dep/string/string.gin")
    ));
}
