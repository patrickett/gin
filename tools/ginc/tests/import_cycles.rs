use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;
use test_fixtures::TempPackage;

#[test]
fn import_cycle_is_a_fatal_flaw() {
    let pkg = TempPackage::new("import_cycle_basic");
    pkg.write("main.gin", "use './a.gin' as a\n\nmain:\n    return 0\n");
    pkg.write("a.gin", "use './b.gin' as b\nx: 1\n");
    pkg.write("b.gin", "use './a.gin' as a\nx: 2\n");

    let exe_path = pkg.join("out.exe");
    let mut args = Args {
        input: pkg.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };

    GinCompiler::compile(&mut args);

    assert!(
        !exe_path.exists(),
        "expected compilation to fail and not produce an executable"
    );
}
