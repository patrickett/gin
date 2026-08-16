use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use analysis::{HoverDocExt, PackageCache};
use diagnostic::Diagnostic;
use flask::{CompileTarget, TargetTriple};
use ginc::driver::{CompilePackage, CompilerDriver, SourceFile, SourcePackage};

fn diagnostic_identity(diagnostic: &Diagnostic) -> (&str, diagnostic::Category, &str) {
    (
        diagnostic.code.slug(),
        diagnostic.category,
        diagnostic.message.as_str(),
    )
}

fn checked_source(
    path: &PathBuf,
    source: &str,
    target: &CompileTarget,
) -> ginc::driver::CheckedPackage {
    let driver = CompilerDriver::default();
    let parsed = driver
        .parse(SourcePackage::new(
            path,
            vec![SourceFile::new(path, source)],
        ))
        .expect("parsed CLI package");
    let mut driver = driver;
    driver.check(
        CompilePackage::new(parsed, target.clone())
            .with_import_resolution(true)
            .with_package_fallback(ast::ty::PackageInstanceKey::workspace("parity", "0")),
    )
}

#[test]
fn cli_and_ide_match_hover_flaws_warnings_target_and_stale_interface_state() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("gin-cli-ide-parity-{suffix}"));
    std::fs::create_dir_all(&root).expect("package directory");
    std::fs::write(
        root.join("flask.jsonc"),
        r#"{"name":"parity","version":"0","authors":[],"target":"wasm32-unknown-unknown"}"#,
    )
    .expect("Flask config");
    let path = root.join("main.gin");
    let source = "index_bits := #index_bits(@())\n#bits(index_bits)\nIndex is in 0...4294967295\nbadName() Index:\n    return missing\n";
    std::fs::write(&path, source).expect("Gin source");
    let target = CompileTarget::Concrete(
        TargetTriple::parse("wasm32-unknown-unknown").expect("target triple"),
    );

    let cli = checked_source(&path, source, &target);
    let cache = PackageCache::for_test();
    cache.add_file(path.clone()).expect("IDE file");
    let ide_diagnostics = cache.package_semantics(std::slice::from_ref(&path))[0]
        .symptoms
        .clone();
    let cli_diagnostics = cli
        .primary_file()
        .front_end_diagnostics()
        .iter()
        .chain(cli.primary_file().typecheck_diagnostics())
        .cloned()
        .collect::<Vec<_>>();
    let mut cli_identity = cli_diagnostics
        .iter()
        .map(diagnostic_identity)
        .collect::<Vec<_>>();
    let mut ide_identity = ide_diagnostics
        .iter()
        .map(diagnostic_identity)
        .collect::<Vec<_>>();
    cli_identity.sort_by(|left, right| left.0.cmp(right.0));
    ide_identity.sort_by(|left, right| left.0.cmp(right.0));
    assert_eq!(cli_identity, ide_identity);
    assert!(
        cli_identity
            .iter()
            .any(|(code, _, _)| *code == "type-unknown-symbol")
    );
    assert!(
        cli_identity
            .iter()
            .any(|(code, _, _)| *code == "parse-bind-name-not-snake-case")
    );

    let hover_byte = source.find("badName() Index").expect("return type hover") + 10;
    let cli_hover = cli
        .primary_file()
        .typed()
        .hover_at_with_package(source, 3, 10, Some(cli.package_index()))
        .map(|hover| {
            ast::HoverDoc::format_markdown(&hover.markdown, hover.module_prefix.as_deref())
        })
        .expect("CLI hover");
    let ide_hover = cache
        .hover(&path, hover_byte as u32)
        .expect("IDE hover")
        .markdown;
    assert_eq!(cli_hover, ide_hover);

    let cli_interface = cli.public_interface();
    let ide_interface = cache
        .public_interface_for(std::slice::from_ref(&path))
        .expect("IDE interface");
    assert_eq!(
        cli_interface.base_fingerprint(),
        ide_interface.base_fingerprint()
    );
    assert_eq!(cli_interface.target_profiles, ide_interface.target_profiles);

    let updated = source.replace("badName", "otherName");
    std::fs::write(&path, &updated).expect("updated Gin source");
    cache.set_contents(&path, updated.clone());
    let updated_cli = checked_source(&path, &updated, &target);
    let updated_ide = cache
        .public_interface_for(std::slice::from_ref(&path))
        .expect("updated IDE interface");
    assert_ne!(
        cli_interface.base_fingerprint(),
        updated_cli.public_interface().base_fingerprint()
    );
    assert_eq!(
        updated_cli.public_interface().base_fingerprint(),
        updated_ide.base_fingerprint()
    );

    std::fs::remove_dir_all(root).expect("remove test package");
}
