use std::collections::HashMap;

use ast::prelude::Intern;
use ast::ty::PackageInstanceKey;
use flask::{CompileTarget, TargetTriple};
use ginc::driver::{CompilePackage, CompilerDriver, SourceFile, SourcePackage};
use test_fixtures::TempPackage;
use typecheck::{BindBody, DefId, TypedExprKind};

#[test]
fn imported_parameterized_record_builder_becomes_checked_assembly() {
    let dependency = TempPackage::new("assembly_builder_dependency");
    dependency.write_flask("builder");
    dependency.write("builder.gin", BUILDER_SOURCE);
    let consumer = TempPackage::new("assembly_builder_consumer");
    consumer.write_flask_with_deps(
        "consumer",
        &format!(
            r#"{{"builder":{{"path":"../{}"}}}}"#,
            dependency
                .path()
                .file_name()
                .expect("temporary dependency has a directory name")
                .to_string_lossy()
        ),
    );
    let main = consumer.join("main.gin");
    let main_source = r#"use builder make

write() Int: #asm(make('gin-asm 1\ntarget "x86_64-unknown-linux-gnu"\ndialect intel\nresult fixed "rax"\ntemplate "nop"'))
"#;

    const BUILDER_SOURCE: &str = r#"String has value Int
Assembly has protocol String

#inline
assembly_new(protocol String) Assembly: return Assembly(protocol: protocol)

#inline
assembly_finish(assembly Assembly) String: return assembly.protocol

#inline
make(protocol String) String: return assembly_finish(assembly_new(protocol))
"#;

    let target = CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap());
    let mut driver = CompilerDriver::default();
    let parsed = driver
        .parse(SourcePackage::new(
            main.clone(),
            vec![SourceFile::new(main.clone(), main_source)],
        ))
        .expect("package parses");
    let package = driver.check(
        CompilePackage::new(parsed, target)
            .with_dependencies({
                let mut dependencies = HashMap::new();
                dependencies.insert("builder".to_string(), dependency.path().to_path_buf());
                dependencies
            })
            .with_import_resolution(true)
            .with_package_fallback(PackageInstanceKey::workspace("assembly-builder-test", "1")),
    );
    assert!(
        !package.has_front_end_flaws(),
        "front-end flaws: {:?}",
        package
            .files()
            .iter()
            .flat_map(|file| file.front_end_diagnostics())
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        !package.has_typecheck_flaws(),
        "typecheck flaws: {:?}",
        package
            .files()
            .iter()
            .flat_map(|file| file.typecheck_diagnostics())
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>()
    );

    let main_file = package.file(&main).expect("checked main file");
    let write = main_file
        .typed()
        .defs
        .get(&DefId(Intern::from_ref("write")))
        .expect("write definition");
    let BindBody::Expr(expr) = write.body else {
        panic!("expected expression body")
    };
    assert!(matches!(
        main_file.typed().exprs.kind_of(expr),
        Some(TypedExprKind::Asm {
            state: typecheck::assembly::AssemblyState::Checked(_),
            ..
        })
    ));
}
