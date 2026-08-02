//! LSP-style hover for an `Architecture` const union (canonical gin_core arch fixture).

use analysis::PackageCache;
use test_fixtures::TempPackage;
use test_fixtures::gin_core::{BOOL_GIN, COPY_GIN, SIZED_GIN, TYPE_GIN};

const ARCH_GIN: &str = r#"--- Target CPU architectures for conditional compilation.
Architecture is 'x86_64'
             or 'arm64'
             or 'wasm32'
"#;

#[test]
fn marker_fixture_imports_sized_trait() {
    let sized_source = SIZED_GIN;
    let file = parser::cursor::TokenCursor::parse_source(sized_source);
    let traits = file.imported_trait_names();
    assert!(
        traits.iter().any(|t| t.as_str() == "Sized"),
        "imports: {:?}",
        traits.iter().map(|t| t.as_str()).collect::<Vec<_>>()
    );
}

fn register_arch_package(
    engine: &mut PackageCache,
    pkg: &TempPackage,
    arch_path: std::path::PathBuf,
    arch_source: &str,
) {
    for (content, rel) in [
        (TYPE_GIN, "reflect/type.gin"),
        (BOOL_GIN, "primitive/bool.gin"),
        (SIZED_GIN, "marker/sized.gin"),
        (COPY_GIN, "marker/copy.gin"),
    ] {
        let p = pkg.write(rel, content);
        engine.add_file(p).unwrap();
    }
    engine.add_file(arch_path.clone()).unwrap();
    engine.set_contents(&arch_path, arch_source.to_string());
}

#[test]
fn lsp_hover_on_architecture_in_package_fixture() {
    let arch_source = ARCH_GIN;
    let pkg = TempPackage::new("architecture");
    pkg.write_flask("core");
    let path = pkg.write("target/arch/arch.gin", arch_source);
    let byte = arch_source
        .find("Architecture is")
        .expect("Architecture declaration") as u32;

    let mut engine = PackageCache::for_test();
    register_arch_package(&mut engine, &pkg, path.clone(), arch_source);

    let hover = engine
        .hover(&path, byte)
        .expect("hover on Architecture in package fixture")
        .markdown;

    assert_eq!(
        hover,
        "```gin\ncore.target.arch\n```\n\n```gin\nArchitecture is 'x86_64'\n             or 'arm64'\n             or 'wasm32'\n```\n---\n\nTarget CPU architectures for conditional compilation."
    );
}

#[test]
fn lsp_hover_on_arch_bind_in_package_fixture() {
    let mut arch_source = String::from(ARCH_GIN);
    arch_source.push_str("\narch Architecture\n");
    let pkg = TempPackage::new("arch_bind");
    pkg.write_flask("core");
    let path = pkg.write("target/arch/arch.gin", &arch_source);
    let byte = arch_source
        .find("arch Architecture")
        .expect("`arch Architecture` in fixture") as u32;

    let mut engine = PackageCache::for_test();
    register_arch_package(&mut engine, &pkg, path.clone(), &arch_source);

    let hover = engine
        .hover(&path, byte)
        .expect("hover on `arch` in package fixture")
        .markdown;

    assert_eq!(
        hover,
        "```gin\ncore.target.arch\n```\n\n```gin\narch Architecture\n```"
    );
}
