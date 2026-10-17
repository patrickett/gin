use flask::GinHome;
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use interface::PublicInterface;
use test_fixtures::TempPackage;

fn compile(pkg: &TempPackage, home: &TempPackage) -> (std::path::PathBuf, PublicInterface) {
    let result = GinCompiler::compile(&mut Args {
        input: pkg.path().to_path_buf(),
        emit: Emit::Interface,
        gin_home: Some(GinHome::new(home.path().to_path_buf())),
        ..Default::default()
    });
    let CompileResult::Success {
        output: Some(output),
    } = result
    else {
        panic!("{result:?}");
    };
    let interface = PublicInterface::decode(&std::fs::read(&output).unwrap()).unwrap();
    (output, interface)
}

#[test]
fn interface_outputs_stay_outside_sources_and_change_with_implementation() {
    let pkg = TempPackage::new("artifact_source");
    let cache = TempPackage::new("artifact_cache");
    pkg.write_flask("pkg");
    pkg.write("main.gin", "#entry\nmain:\n    return 1\n");
    let (first, _) = compile(&pkg, &cache);
    assert!(first.starts_with(cache.path()));
    assert!(!pkg.join("target").exists());
    assert_eq!(std::fs::read_dir(pkg.path()).unwrap().count(), 2);
    pkg.write("main.gin", "#entry\nmain:\n    return 2\n");
    let (second, _) = compile(&pkg, &cache);
    assert_ne!(first, second);
    assert!(first.exists());
}

#[test]
fn public_type_changes_are_visible_in_interfaces() {
    let pkg = TempPackage::new("interface_types");
    let cache = TempPackage::new("interface_type_cache");
    pkg.write_flask("pkg");
    pkg.write("types.gin", "Choice is Left or Right\n");
    let (_, old) = compile(&pkg, &cache);
    assert!(!old.declarations.is_empty());
    pkg.write("types.gin", "Choice is Left or Right or Middle\n");
    let (_, new) = compile(&pkg, &cache);
    assert_eq!(
        interface::classify_interfaces(&old, &new),
        interface::Compatibility::Breaking
    );
}

#[test]
fn storage_ownership_changes_are_visible_in_interfaces() {
    let pkg = TempPackage::new("interface_storage_ownership");
    let cache = TempPackage::new("interface_storage_ownership_cache");
    pkg.write_flask("pkg");
    let source = "Element is Ready\nStorage has\n    address @Element\n    finish(self):\n        Storage(address) := self\n        raw: address as @Element\n        return\n";
    pkg.write("types.gin", source);
    let (_, old) = compile(&pkg, &cache);
    pkg.write(
        "types.gin",
        &source.replace("address @Element", "address own Element"),
    );
    let (_, new) = compile(&pkg, &cache);
    assert_eq!(
        interface::classify_interfaces(&old, &new),
        interface::Compatibility::Breaking
    );
}

#[cfg(unix)]
#[test]
fn read_only_module_directories_can_emit_interfaces() {
    use std::os::unix::fs::PermissionsExt;
    let pkg = TempPackage::new("readonly_sources");
    let cache = TempPackage::new("readonly_cache");
    pkg.write_flask("pkg");
    pkg.write("main.gin", "#entry\nmain:\n    return\n");
    let original = std::fs::metadata(pkg.path()).unwrap().permissions();
    std::fs::set_permissions(pkg.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = GinCompiler::compile(&mut Args {
        input: pkg.path().to_path_buf(),
        emit: Emit::Interface,
        gin_home: Some(GinHome::new(cache.path().to_path_buf())),
        ..Default::default()
    });
    std::fs::set_permissions(pkg.path(), original).unwrap();
    assert!(matches!(result, CompileResult::Success { output: Some(_) }));
    assert_eq!(std::fs::read_dir(pkg.path()).unwrap().count(), 2);
}

#[test]
fn native_objects_and_executables_use_the_shared_cache() {
    let pkg = TempPackage::new("native_sources");
    let cache = TempPackage::new("native_cache");
    pkg.write_flask("pkg");
    let input = pkg.write("main.gin", "#entry\nmain:\n    return 0\n");
    for emit in [Emit::Obj, Emit::Exe] {
        let result = GinCompiler::compile(&mut Args {
            input: input.clone(),
            emit,
            target: Some(test_fixtures::host_target_triple()),
            gin_home: Some(GinHome::new(cache.path().to_path_buf())),
            ..Default::default()
        });
        let CompileResult::Success {
            output: Some(output),
        } = result
        else {
            panic!("{result:?}");
        };
        assert!(output.starts_with(cache.path()));
        assert!(output.is_file());
        if emit == Emit::Exe {
            assert!(
                std::process::Command::new(output)
                    .status()
                    .unwrap()
                    .success()
            );
        }
    }
    assert_eq!(std::fs::read_dir(pkg.path()).unwrap().count(), 2);
}

#[test]
fn object_reuse_validates_content_and_respects_no_cache() {
    let pkg = TempPackage::new("object_reuse_source");
    let cache = TempPackage::new("object_reuse_cache");
    pkg.write_flask("pkg");
    let mut args = Args {
        input: pkg.write("main.gin", "#entry\nmain:\n    return 0\n"),
        emit: Emit::Obj,
        target: Some(test_fixtures::host_target_triple()),
        gin_home: Some(GinHome::new(cache.path().to_path_buf())),
        ..Default::default()
    };
    let CompileResult::Success {
        output: Some(output),
    } = GinCompiler::compile(&mut args)
    else {
        panic!("initial compilation failed");
    };
    let original = std::fs::read(&output).unwrap();
    let receipt = output.with_extension("o.sha256");
    assert!(receipt.is_file());
    let recorded = std::fs::metadata(&receipt).unwrap().modified().unwrap();
    assert!(matches!(
        GinCompiler::compile(&mut args),
        CompileResult::Success { .. }
    ));
    assert_eq!(
        recorded,
        std::fs::metadata(&receipt).unwrap().modified().unwrap()
    );
    std::fs::write(&output, "corrupt").unwrap();
    assert!(matches!(
        GinCompiler::compile(&mut args),
        CompileResult::Success { .. }
    ));
    assert_eq!(std::fs::read(&output).unwrap(), original);
    std::fs::write(&receipt, "invalid receipt").unwrap();
    args.no_cache = true;
    assert!(matches!(
        GinCompiler::compile(&mut args),
        CompileResult::Success { .. }
    ));
    assert_eq!(
        std::fs::read_to_string(&receipt).unwrap(),
        "invalid receipt"
    );
    assert_eq!(std::fs::read(&output).unwrap(), original);
}
