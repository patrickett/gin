use std::collections::HashMap;

use ast::ty::PackageInstanceKey;
use flask::CompileTarget;
use resolve::{PackageFrontEndOptions, ParsedFile, ParsedModuleCache, run_package_front_end};
use test_fixtures::TempPackage;
use typecheck::transform::PackageTransformOptions;

fn flaws(package: &TempPackage, source: &str) -> Vec<diagnostic::Diagnostic> {
    package.write("main.gin", source);
    let path = package.path().canonicalize().unwrap().join("main.gin");
    let output = run_package_front_end(
        vec![ParsedFile::read(&path).unwrap()],
        &HashMap::new(),
        ParsedModuleCache::default(),
        PackageFrontEndOptions {
            gin_home: None,
            compile_target: CompileTarget::Library,
            transform: PackageTransformOptions::FULL,
            resolve_imports: true,
            package_fallback: PackageInstanceKey::workspace("fixture", "0"),
            transform_paths: None,
        },
    );
    let mut flaws = Vec::new();
    for file in output.files {
        assert!(
            file.parsed.output.symptoms.is_empty(),
            "{}: {:?}",
            file.parsed.path.display(),
            file.parsed.output.symptoms
        );
        assert!(
            file.import_diagnostics.is_empty(),
            "{:?}",
            file.import_diagnostics
        );
        assert!(
            file.prepare_diagnostics.is_empty(),
            "{:?}",
            file.prepare_diagnostics
        );
        let typed = file.typed.unwrap();
        let diagnostics = typed.all_flaws();
        if file.parsed.path != path {
            assert!(
                diagnostics.is_empty(),
                "{}: {diagnostics:?}",
                file.parsed.path.display()
            );
        }
        flaws.extend(diagnostics.into_iter().map(|(_, flaw)| flaw.clone()));
    }
    flaws
}

#[test]
fn imported_instruction_capacity_proves_writes_and_successive_emissions() {
    let package = TempPackage::new("imported-instruction-capacity");
    package.write_flask("fixture");
    package.write(
        "range/range.gin",
        include_str!("../../../modules/core/range/range.gin"),
    );
    package.write(
        "target/emission/emission.gin",
        &include_str!("../../../modules/core/target/emission/emission.gin")
            .replace("use core.range Range", "use '../../range' Range"),
    );
    for (path, source) in [
        (
            "target/x86_64/registers.gin",
            include_str!("../../../modules/core/target/x86_64/registers.gin"),
        ),
        (
            "target/aarch64/registers.gin",
            include_str!("../../../modules/core/target/aarch64/registers.gin"),
        ),
    ] {
        package.write(
            path,
            &source.replace("use core.target.emission", "use '../emission'"),
        );
    }
    let imports = concat!(
        "use './target/emission' Instructions\n",
        "use './target/x86_64' Syscall\n",
        "use './target/aarch64' SupervisorCall, CopyRegister, NegateCarry, x0, x1, x17\n",
    );
    let source = format!(
        "{imports}\
append_x86(buffer mut Instructions and buffer.length = 1022):
    Syscall.emit(buffer)
    return thus buffer.length = 1024
append_arm(buffer mut Instructions and buffer.length = 1020):
    SupervisorCall.emit(buffer, 0)
    return thus buffer.length = 1024
chain Instructions:
    instructions: Instructions.new
    CopyRegister.emit(instructions, x17, x1)
    SupervisorCall.emit(instructions, 128)
    NegateCarry.emit(instructions, x0)
    CopyRegister.emit(instructions, x1, x17)
    return instructions thus instructions.length = 16
"
    );
    let valid_flaws = flaws(&package, &source);
    assert!(valid_flaws.is_empty(), "{valid_flaws:?}");
    for (operation, start, arguments) in [
        ("Syscall", 1023, "buffer"),
        ("SupervisorCall", 1021, "buffer, 0"),
    ] {
        let source = format!(
            "{imports}\
overflow(buffer mut Instructions and buffer.length = {start}):
    {operation}.emit({arguments})
    return
"
        );
        let flaws = flaws(&package, &source);
        assert!(!flaws.is_empty(), "{source}");
        assert!(
            flaws
                .iter()
                .all(|flaw| flaw.code.slug().starts_with("type-parameter-refinement-")),
            "{flaws:?}"
        );
    }
}

#[test]
fn imported_constants_do_not_replace_parameter_names_or_unknown_names() {
    let package = TempPackage::new("imported-constant-shadowing");
    package.write_flask("fixture");
    package.write("bounds/bounds.gin", "capacity := 16\n");
    for (arguments, accepted) in [("4, 3", true), ("4, 10", false)] {
        let source = format!(
            "use './bounds' capacity
bounded(capacity in 0 to 100, value in 0 to 100 and value < capacity): return
call: bounded({arguments})
"
        );
        let flaws = flaws(&package, &source);
        assert_eq!(flaws.is_empty(), accepted, "{flaws:?}");
    }
    let source = "use './bounds' capacity as imported
bounded(value in 0 to 100):
    local is in 0 to capacity: value
    return local
";
    let flaws = flaws(&package, source);
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-integer-refinement-requires-finite-bounds"),
        "{flaws:?}"
    );
}
