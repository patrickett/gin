use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::TempPackage;

#[test]
fn generated_vec_accepts_default_explicit_and_reordered_allocators() {
    let package = TempPackage::new("generated_vec_generic_arguments");
    let dependency = TempPackage::new("vec_dependency");
    let target = test_fixtures::host_target_triple();
    dependency.write(
        "flask.jsonc",
        "{\"name\":\"foundation\",\"version\":\"0.0.0\",\"authors\":[]}",
    );
    dependency.write("primitive/int.gin", "Byte is in 0 to 255\nType is Unit\n");
    dependency.write(
        "alloc/count.gin",
        "use '../primitive' Byte\n#phantom(unit)\nCount(unit) is in 0 to 4294967295\n",
    );
    dependency.write(
        "alloc/allocator.gin",
        "use Count\nAllocation(element) has pointer @element, capacity Count(element)\nGlobalAllocator is ()\n",
    );
    dependency.write(
        "collection/vec.gin",
        "use '../alloc' Allocation, Count, GlobalAllocator\nVec(element Type, allocator: GlobalAllocator) has storage Allocation(element), length Count(element)\n",
    );
    let dependency_name = dependency
        .path()
        .file_name()
        .expect("temporary dependency directory name")
        .to_string_lossy();
    let dependency_path = format!("../{dependency_name}");

    package.write(
        "flask.jsonc",
        &format!(
            r#"{{
  "name": "generated_vec_generic_arguments",
  "version": "0.0.0",
  "authors": [],
  "target": "{target}",
  "dependencies": {{
    "foundation": {{ "path": "{dependency_path}" }}
  }}
}}"#,
        ),
    );
    package.write(
        "main.gin",
        "use foundation.collection Vec\n\
\n\
ArenaAllocator has marker Int\n\
\n\
default_values(value Vec(Int)) Vec(Int): value\n\
explicit_values(value Vec(Int, allocator: ArenaAllocator)) Vec(Int, allocator: ArenaAllocator): value\n\
reordered_values(value Vec(allocator: ArenaAllocator, element: Int)) Vec(allocator: ArenaAllocator, element: Int): value\
",
    );

    let output = package.join("generated_vec_generic_arguments.ginif");
    let mut args = Args {
        input: package.path().to_path_buf(),
        output: Some(output),
        emit: Emit::Interface,
        profile: Profile::Debug,
        target: Some(target),
        no_cache: true,
        ..Default::default()
    };

    let result = GinCompiler::compile(&mut args);
    assert!(
        result.is_success(),
        "generated Vec generic argument package should compile: {result:?}"
    );
    assert!(
        !result.has_typecheck_diagnostics(),
        "generated Vec generic argument package should be diagnostic-free: {result:?}"
    );
}
